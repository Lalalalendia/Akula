#!/usr/bin/env python3
"""Research-only comparison of Akula's RGBA, dilation, splice and exact-match ports."""
import ctypes as c
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import time

ROOT=Path(__file__).resolve().parent
OUT=Path("akula-kernel-results").resolve()
U=c.c_uint8
W=c.c_uint32
N=c.c_size_t
PU=c.POINTER(U)
PW=c.POINTER(W)
NAMES=("c","cpp","rust","zig")

def arr(buf): return (U*max(1,len(buf))).from_buffer_copy(buf or b"\0")
def val(buf,n): return c.string_at(buf,n)
def rgba(w,h):
    n=w*h*4
    return (bytes(range(256))*((n+255)//256))[:n]
def mask(w,h):
    n=w*h
    unit=bytes((i*73+i//7*31)&255 for i in range(256))
    return (unit*((n+255)//256))[:n]

def ref_half(src,w,h):
    if (w,h)==(1,1) or w==0 or h==0:return (0,b"")
    dw,dh=(w+1)//2,(h+1)//2
    out=bytearray(dw*dh*4); translucent=False
    for y in range(dh):
        for x in range(dw):
            xs=(2*x,min(2*x+1,w-1));ys=(2*y,min(2*y+1,h-1))
            at=[4*(yy*w+xx) for yy in ys for xx in xs]
            for ch in range(4):
                k=(sum(src[i+ch] for i in at)+2)//4
                out[4*(y*dw+x)+ch]=k
                if ch==3 and k!=255:translucent=True
    return (2 if translucent else 1,bytes(out))

def ref_dilate(src,w,h):
    tmp=bytearray(w*h);dst=bytearray(w*h)
    for y in range(h):
        for x in range(w):
            i=y*w+x
            tmp[i]=max(src[i],src[y*w+max(0,x-1)],src[y*w+min(w-1,x+1)])
    for y in range(h):
        for x in range(w):
            i=y*w+x
            dst[i]=max(tmp[max(y-1,0)*w+x],tmp[i],tmp[min(h-1,y+1)*w+x])
    return bytes(dst)

def ref_splice(a,b):
    common=min(len(a),len(b));pre=0
    while pre<common and a[pre]==b[pre]:pre+=1
    suf=0
    while suf<common-pre and a[len(a)-1-suf]==b[len(b)-1-suf]:suf+=1
    inserted=b[pre:len(b)-suf]
    return pre.to_bytes(4,"little")+(len(a)-pre-suf).to_bytes(4,"little")+len(inserted).to_bytes(4,"little")+inserted

def ref_search(text,words):
    pats=[s.encode() for s in words];out=[];i=0
    while i<len(text):
        word=next((p for p in pats if text.startswith(p,i)),None)
        if word:out.append((i,i+len(word)));i+=len(word)
        else:
            b=text[i];i+=1 if b<128 else 2 if b<224 else 3 if b<240 else 4
    return out

def binding(path):
    lib=c.CDLL(str(path))
    desc=[
      ("ak_mipmap",W,[PU,PU,N,N]),
      ("ak_dilate",None,[PU,PU,PU,N,N]),
      ("ak_splice",N,[PU,N,PU,N,PU]),
      ("ak_apply",N,[PU,N,PU,N,PU]),
      ("ak_search",N,[PU,N,PU,PW,N,PW,N])]
    for name,restype,args in desc:
        f=getattr(lib,name);f.argtypes=args;f.restype=restype
    return lib

def image(w,h):
    src=rgba(w,h);k=(w+1)//2*((h+1)//2)*4
    return {"w":w,"h":h,"source":src,"a":arr(src),"dst":(U*max(k,1))(),"n":k}
def morph(w,h):
    src=mask(w,h);n=w*h
    return {"w":w,"h":h,"source":src,"a":arr(src),"tmp":(U*max(n,1))(),"dst":(U*max(n,1))(),"n":n}
def delta(old,new):
    return {"old":old,"new":new,"a":arr(old),"b":arr(new),"patch":(U*(12+len(new)))(),
            "out":(U*max(len(new),1))(),"bn":len(old),"tn":len(new)}
def search(text,words):
    parts=[p.encode() for p in words];ends=[0]
    for part in parts:ends.append(ends[-1]+len(part))
    return {"source":text,"a":arr(text),"needles":arr(b"".join(parts)),
      "starts":(W*len(ends))(*ends),"count":len(parts),"n":len(text),
      "hits":(W*(2*max(len(text),1)))(),"cap":max(1,len(text)),"words":words}

def invoke(fn,kind,x,full):
    if kind=="rgba":
        flag=fn.ak_mipmap(x["a"],x["dst"],x["w"],x["h"])
        return (flag,val(x["dst"],x["n"]) if flag else b"") if full else flag
    if kind=="dilate":
        fn.ak_dilate(x["a"],x["tmp"],x["dst"],x["w"],x["h"])
        return val(x["dst"],x["n"]) if full else None
    if kind=="splice":
        n=fn.ak_splice(x["a"],x["bn"],x["b"],x["tn"],x["patch"])
        return val(x["patch"],n) if full else n
    if kind=="apply":
        n=fn.ak_apply(x["a"],x["bn"],x["patch"],12+len(x["new"]),x["out"])
        return val(x["out"],n) if full else n
    if kind=="search":
        n=fn.ak_search(x["a"],x["n"],x["needles"],x["starts"],
                       x["count"],x["hits"],x["cap"])
        if n>x["cap"]:raise AssertionError("invalid match count")
        return [(x["hits"][2*i],x["hits"][2*i+1]) for i in range(n)] if full else n
    raise ValueError(kind)

def small_checks(libs):
    checks=[]
    for w,h in ((1,1),(1,2),(2,1),(3,3),(7,5),(64,37)):
        x=image(w,h);want=ref_half(x["source"],w,h)
        for name,fn in libs.items():
            if invoke(fn,"rgba",x,True)!=want:raise AssertionError(f"RGBA parity {name} {w}x{h}")
        checks.append(f"rgba {w}x{h}")
    for w,h in ((1,1),(1,17),(17,1),(3,3),(7,5),(64,37)):
        x=morph(w,h);want=ref_dilate(x["source"],w,h)
        for name,fn in libs.items():
            if invoke(fn,"dilate",x,True)!=want:raise AssertionError(f"dilate parity {name} {w}x{h}")
        checks.append(f"dilate {w}x{h}")
    pairs=[(b"",b""),(b"a",b"b"),(b"abc",b"abc"),(b"abc",b"abcd"),
       (b"abcd",b"abc"),(b"abcdef",b"abZZef"),
       ("глава 🐟".encode(),"глава 🐠!".encode())]
    for old,new in pairs:
        x=delta(old,new);want=ref_splice(old,new)
        for name,fn in libs.items():
            got=invoke(fn,"splice",x,True)
            if got!=want:raise AssertionError(f"splice parity {name}")
            if invoke(fn,"apply",x,True)!=new:raise AssertionError(f"apply parity {name}")
        checks.append(f"splice and apply {len(old)}->{len(new)}")
    words=sorted({"alphabet","alpha","gamma","beta","сеть","а"},key=lambda s:(-len(s),s))
    for text in (b"",b"alpha alphabet beta alphabeta gamma",
       "alpha сеть альфа сеть gamma а beta".encode(),
       "🐟 alpha 📚 alphabet a beta сеть".encode()):
        x=search(text,words);want=ref_search(text,words)
        for name,fn in libs.items():
            if invoke(fn,"search",x,True)!=want:raise AssertionError(f"search parity {name}")
        checks.append(f"search {len(text)} bytes {len(want)} matches")
    return checks

def cases():
    words=sorted({"alphabet","alpha","gamma","beta","сеть","а"},key=lambda s:(-len(s),s))
    block="alpha alphabet beta gamma сеть 📚 документ alpha гамма beta : ".encode()
    text=block*(1_500_000//len(block))
    paragraph="Глава 42. Красный код. Документ изменён.\n".encode()
    old=(paragraph*(1_600_000//len(paragraph)+1))[:1_600_000]
    new=old[:750_000]+b"<revision>"*4200+old[801_000:]
    return {
      "rgba_1281x885":("rgba",image(1281,885)),
      "rgba_4096x2160":("rgba",image(4096,2160)),
      "dilate_2048x2048":("dilate",morph(2048,2048)),
      "splice_1.6MB":("splice",delta(old,new)),
      "apply_1.6MB":("apply",delta(old,new)),
      "search_utf8_1.5MB":("search",search(text,words))}

def large_checks(libs,tests):
    checks=[]
    for case,(kind,x) in tests.items():
        if kind in ("splice","apply"):
            want=ref_splice(x["old"],x["new"])
            if kind=="apply":
                c.memmove(x["patch"],want,len(want))
                want=x["new"]
        else:want=invoke(libs["c"],kind,x,True)
        for name,fn in libs.items():
            if invoke(fn,kind,x,True)!=want:
                raise AssertionError(f"LARGE BYTE PARITY FAILED: {case} {name}")
        checks.append(dict(case=case,kind=kind,parity="exact",oracle="Python" if kind in ("splice","apply") else "C after independent Python small fixtures"))
    return checks

def measure(libs,tests,rounds):
    rng=random.Random(20261025);result=[]
    for name,(kind,x) in tests.items():
        windows={k:[] for k in libs}
        counts={}
        for k,fn in libs.items():
            invoke(fn,kind,x,False)
            t=time.perf_counter_ns()
            invoke(fn,kind,x,False)
            per_call=max(time.perf_counter_ns()-t,1000)
            counts[k]=min(1500,max(1,round(40_000_000/per_call)))
        for r in range(rounds+2):
            order=list(libs);rng.shuffle(order)
            for k in order:
                t=time.perf_counter_ns();anti=0
                for _ in range(counts[k]):
                    answer=invoke(libs[k],kind,x,False)
                    if answer is not None:anti^=int(answer)
                elapsed=(time.perf_counter_ns()-t)/counts[k]/1_000_000
                if r>=2:windows[k].append(elapsed)
                if anti<0:raise AssertionError("unreachable")
        for k,values in windows.items():
            values.sort()
            result.append(dict(case=name,kernel=kind,language=k,rounds=len(values),
                calls_per_round=counts[k],median_ms=round(statistics.median(values),6),
                p95_ms=round(values[math.ceil(.95*len(values))-1],6),
                samples_ms=[round(t,6) for t in values]))
    return result

def build():
    OUT.mkdir(parents=True,exist_ok=True)
    cmds={
      "c":["gcc","-std=c11","-O3","-shared","-fPIC",str(ROOT/"kernels.c"),"-o",str(OUT/"c.so")],
      "cpp":["g++","-std=c++17","-O3","-x","c++","-shared","-fPIC",str(ROOT/"kernels.c"),"-o",str(OUT/"cpp.so")],
      "rust":["rustc","--edition=2021","--crate-type=cdylib","-C","opt-level=3","-C","target-cpu=x86-64",str(ROOT/"kernels.rs"),"-o",str(OUT/"rust.so")],
      "zig":["zig","build-lib",str(ROOT/"kernels.zig"),"-dynamic","-O","ReleaseFast","-target","x86_64-linux-gnu",f"-femit-bin={OUT/'zig.so'}"]}
    records={}
    for k,cmd in cmds.items():
        start=time.perf_counter()
        p=subprocess.run(cmd,capture_output=True,text=True,timeout=240)
        (OUT/f"compile-{k}.log").write_text(p.stdout+"\n"+p.stderr)
        if p.returncode!=0:raise RuntimeError(f"BUILD FAILED {k}: {p.stderr[-1800:]}")
        source=ROOT/("kernels.c" if k in ("c","cpp") else "kernels.rs" if k=="rust" else "kernels.zig")
        ver=subprocess.run([cmd[0],"--version"],capture_output=True,text=True,timeout=10)
        records[k]=dict(command=cmd,elapsed_s=round(time.perf_counter()-start,3),
          library_bytes=(OUT/f"{k}.so").stat().st_size,
          source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
          version=(ver.stdout+ver.stderr).splitlines()[:2])
    return records

def main():
    metadata=build()
    libs={k:binding(OUT/f"{k}.so") for k in NAMES}
    small=small_checks(libs);tests=cases();large=large_checks(libs,tests)
    timings=measure(libs,tests,12)
    cpu="unknown"
    try:cpu=next(x.split(":",1)[1].strip() for x in Path("/proc/cpuinfo").read_text().splitlines() if x.startswith("model name"))
    except(OSError,StopIteration):pass
    env=dict(cpu=cpu,os=platform.platform(),sha=os.getenv("GITHUB_SHA"),rounds=12,
     source_sha="641a437005132a4520b3606586d11d112f19fd5a",
     warning="Standalone Akula kernel ports. Not installed binaries. Shared-Python process so per-language RSS is not measured. Search is Exact/Substring only.")
    report=dict(environment=env,builds=metadata,small_checks=small,large_checks=large,results=timings)
    (OUT/"results.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    with (OUT/"results.csv").open("w",newline="") as f:
        writer=csv.DictWriter(f,fieldnames=["case","kernel","language","rounds","calls_per_round","median_ms","p95_ms"],extrasaction="ignore")
        writer.writeheader();writer.writerows(timings)
    lines=[f"# Akula kernels, 4 languages",f"CPU: {cpu}; SHA: {env['sha']}",
       f"Small independent fixtures: {len(small)}. Large parity cases: {len(large)}. Measured rounds: 12.",
       "", "| Case | C ms | C++ ms | Rust ms | Zig ms |","|---|---:|---:|---:|---:|"]
    for key in tests:
        row={r["language"]:r for r in timings if r["case"]==key}
        lines.append("| "+key+" | "+" | ".join(str(row[k]["median_ms"]) for k in NAMES)+" |")
    lines+=["","| Language | Compile s | Shared library, bytes |","|---|---:|---:|"]
    for k in NAMES:lines.append(f"| {k} | {metadata[k]['elapsed_s']} | {metadata[k]['library_bytes']} |")
    lines+=["","C++ compiles the same C-compatible source as C. Full-source language comparison and real UI speed are NOT established. These are isolated algorithm ports, not full compiled Akula. Source parity excludes Unicode folding and complete secure delta frames. Compiler time excludes installation; runner memory is shared.", ""]
    (OUT/"summary.md").write_text("\n".join(lines))
    print("\n".join(lines))

if __name__=="__main__":main()
