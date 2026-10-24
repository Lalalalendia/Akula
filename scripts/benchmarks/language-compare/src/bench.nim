const Limit = 10_000_000
var marked = newSeq[bool](Limit + 1)
var p = 2
while p*p <= Limit:
  if not marked[p]:
    var j = p*p
    while j <= Limit:
      marked[j] = true
      j += p
  inc p
var count = 0
for i in 2..Limit:
  if not marked[i]: inc count
var h: uint32 = 2166136261'u32
for i in 0'u32..<20_000_000'u32:
  h = (h xor ((i *% 131'u32 +% 17'u32) and 255'u32)) *% 16777619'u32
echo count
echo h
