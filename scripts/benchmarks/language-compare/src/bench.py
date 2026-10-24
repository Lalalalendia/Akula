LIMIT = 10_000_000
marked = bytearray(LIMIT + 1)
p = 2
while p * p <= LIMIT:
    if not marked[p]:
        for j in range(p * p, LIMIT + 1, p):
            marked[j] = 1
    p += 1
count = sum(1 for i in range(2, LIMIT + 1) if not marked[i])
h = 2166136261
for i in range(20_000_000):
    h = ((h ^ ((i * 131 + 17) & 255)) * 16777619) & 0xFFFFFFFF
print(count)
print(h)
