# Corpus benchmark

Pinned upstream manifest: https://github.com/imazen/imazen-26/blob/36da2bfbd35fe9f816ecdfa47070daf2685af64e/manifests/test.tsv

Use `bash scripts/image-bench/run-corpus.sh`. Only allowlisted JPEG and PNG images are fetched; lengths, dimensions, HTTPS origin, SHA256 are verified. Files are stored outside Git. Five canonical test-split images (1019,1059,5007,7007,9007) plus optional 50 MP 1407.

Results are **preview path read+decode+resize**, not Google Criterion decode-only. PNG comparator is absent (marked not_jpeg), AVIF/WebP not tested. Do not extrapolate quality from speed. This is research-only, not a security sandbox.
