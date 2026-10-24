#!/usr/bin/env python3
import hashlib,json,os,pathlib
import boto3
endpoint="http://127.0.0.1:9000"
client=boto3.client("s3",endpoint_url=endpoint,aws_access_key_id="akula_ci",aws_secret_access_key="akula_ci_test_only",region_name="us-east-1")
bucket="akula-cdn-test";key="synthetic-64m.bin"
size=64*1024*1024;chunk=256*1024
client.create_bucket(Bucket=bucket)
block=bytes(i%251 for i in range(chunk))
parts=[]
h=hashlib.sha256()
with pathlib.Path("minio-fixture.bin").open("wb") as f:
 for i in range(size//chunk):
  f.write(block);h.update(block)
  parts.append({"offset":i*chunk,"size":chunk,"sha256":hashlib.sha256(block).hexdigest()})
client.upload_file("minio-fixture.bin",bucket,key)
origin=f"{endpoint}/{bucket}/{key}"
data={"version":1,"content_id":h.hexdigest(),"name":"synthetic-64m.bin","size":size,"chunk_size":chunk,"origin_url":origin,"chunks":parts}
raw=json.dumps(data,separators=(",",":"),ensure_ascii=False).encode()
pathlib.Path("minio-manifest.json").write_bytes(raw)
with open(os.environ["GITHUB_ENV"],"a") as f:
 f.write("AKULA_TEST_MANIFEST=minio-manifest.json\n")
 f.write("AKULA_TEST_MANIFEST_SHA256="+hashlib.sha256(raw).hexdigest()+"\n")
print("MinIO fixture uploaded",size,"bytes",len(parts),"chunks")
