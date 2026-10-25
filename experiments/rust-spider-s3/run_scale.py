"""Scale test for approved URL spool: all objects verified on real local MinIO."""
import argparse, json, os, signal, statistics, subprocess, sys, tempfile, uuid
from pathlib import Path
from fixtures import generate
from run_benchmark import prepare_bucket, wait_for_minio, wait_for_http, verify, free_port, run_process, assert_success

HERE=Path(__file__).resolve().parent
def trial(binary, root, count, size, rounds, bucket, dest):
    dest.mkdir(parents=True,exist_ok=True)
    manifest=generate(root,count,size)
    port=free_port()
    origin=f"http://127.0.0.1:{port}"
    server=subprocess.Popen([sys.executable,"-m","http.server",str(port),"--bind","127.0.0.1","--directory",str(root)],
                            stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
    reports={"pages":count,"status":"RUNNING","python":[],"rust":[],"recovery":{}}
    try:
        wait_for_http(origin+"/index.html")
        common=dict(os.environ,TRIAL_ROOT=origin+"/index.html",TRIAL_URLS_FILE=str(root/"urls.tsv"),
                    TRIAL_PAGES=str(count),TRIAL_MAX_PAGE_BYTES=str(size+2048),S3_BUCKET=bucket)
        uid=os.environ.get("GITHUB_RUN_ID","local")+"-"+uuid.uuid4().hex[:9]
        def execute(mode,prefix,crash,log):
            env=dict(common,S3_PREFIX=prefix,CRASH_AFTER_PUT_N=str(crash))
            cmd=[sys.executable,str(HERE/"python_crawler.py")] if mode=="python" else [str(binary)]
            return run_process(cmd,env,dest/log,timeout=900)
        for i in range(rounds):
            for mode in (("python","rust") if i%2==0 else ("rust","python")):
                prefix=f"bounded/{uid}/{count}/clean/{i}/{mode}"
                metric=execute(mode,prefix,0,f"clean-{i}-{mode}.log")
                assert_success(metric)
                metric["integrity"]=verify(bucket,prefix,manifest)
                reports[mode].append(metric)
                print("CLEAN",count,mode,metric["wall_s"],metric["sampled_peak_rss_mib"],flush=True)
        for mode in (("python","rust") if count<=1000 else ("rust",)):
            prefix=f"bounded/{uid}/{count}/recovery/{mode}"
            crashed=execute(mode,prefix,18,f"crash-{mode}.log")
            if crashed["returncode"]!=87:raise AssertionError(f"crash not triggered {mode}: {crashed}")
            before=verify(bucket,prefix,manifest,complete=False)
            if not 0<before["objects"]<count:raise AssertionError("invalid partial")
            resumed=execute(mode,prefix,0,f"resume-{mode}.log")
            assert_success(resumed)
            after=verify(bucket,prefix,manifest)
            reports["recovery"][mode]={"before":before,"after":after,"crash_returncode":87}
        reports["summary"]={mode:{"median_wall_s":round(statistics.median(x["wall_s"] for x in reports[mode]),4),
                                  "median_rss_mib":round(statistics.median(x["sampled_peak_rss_mib"] for x in reports[mode]),2)}
                              for mode in ("python","rust")}
        reports["status"]="PASSED"
        return reports
    except Exception as exc:
        reports["status"]="FAILED";reports["error"]=repr(exc)
        raise
    finally:
        if server.poll() is None:
            os.killpg(server.pid,signal.SIGTERM);server.wait(timeout=10)
        (dest/"results.json").write_text(json.dumps(reports,indent=2),encoding="utf-8")

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--bounded-binary",required=True,type=Path)
    ap.add_argument("--out",default=HERE/"artifacts"/"scale",type=Path)
    ap.add_argument("--rounds",type=int,default=1)
    ap.add_argument("--profile",choices=("smoke","full"),default="full")
    opts=ap.parse_args()
    binary=opts.bounded_binary.resolve()
    if not binary.is_file():ap.error(f"binary missing {binary}")
    if not 1<=opts.rounds<=5:ap.error("rounds 1..5")
    wait_for_minio()
    prepare_bucket("spider-benchmark")
    profiles=[(20,2048)] if opts.profile=="smoke" else [(1000,8192),(10000,1024)]
    results={}
    try:
        for pages,size in profiles:
            with tempfile.TemporaryDirectory() as d:
                results[str(pages)]=trial(binary,Path(d),pages,size,opts.rounds,
                                           "spider-benchmark",opts.out/str(pages))["summary"]
    finally:
        opts.out.mkdir(parents=True,exist_ok=True)
        (opts.out/"index.json").write_text(json.dumps(results,indent=2),encoding="utf-8")
    print(json.dumps(results,indent=2))
if __name__=="__main__":main()
