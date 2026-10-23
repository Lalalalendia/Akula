#!/usr/bin/env python3
"""Bounded-rate real loopback QUIC smoke. Not a physical-link throughput guarantee."""
import pathlib, subprocess, tempfile, time, select, re, argparse
def invoke(binary,*args):
 p=subprocess.run([str(binary),*map(str,args)],capture_output=True,text=True,timeout=35)
 if p.returncode: raise AssertionError(f"{p.args}: {p.stderr}")
 return p.stdout
def main():
 ap=argparse.ArgumentParser();ap.add_argument("--binary",required=True);a=ap.parse_args();binary=pathlib.Path(a.binary).resolve()
 with tempfile.TemporaryDirectory() as t:
  d=pathlib.Path(t);sk=d/"s.key";ck=d/"c.key"
  sid=invoke(binary,"identity","--key",sk).split()[-1]
  cid=invoke(binary,"identity","--key",ck).split()[-1]
  book=d/"test.bin";book.write_bytes(bytes((i*17)%256 for i in range(262144)))
  p=subprocess.Popen([str(binary),"serve","--key",str(sk),"--book",str(book),"--allow",cid,
   "--quota-state",str(d/"ledger"),"--global-rate","65536","--peer-rate","65536"],
   stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,bufsize=1)
  try:
   ready,_,_=select.select([p.stdout],[],[],30); assert ready,"no listener"
   line=p.stdout.readline().strip();m=re.search(r"addr=([^ ]+)",line);assert m,line
   start=time.monotonic()
   out=invoke(binary,"fetch","--key",ck,"--remote-id",sid,"--remote-addr",m[1],
      "--stage",d/"stage","--output",d/"result.bin")
   elapsed=time.monotonic()-start
   assert "COMPLETE" in out,out
   assert (d/"result.bin").read_bytes()==book.read_bytes()
   assert elapsed>=3.4, f"rate limit bypassed: {elapsed:.3f}s"
   assert elapsed<25, f"unreasonably slow: {elapsed:.3f}s"
   print(f"PASS paced QUIC transfer: {elapsed:.3f}s for 262144 bytes at 65536 B/s")
  finally:
   p.terminate()
   try:p.communicate(timeout=5)
   except subprocess.TimeoutExpired:p.kill();p.communicate(timeout=5)
if __name__=="__main__":main()
