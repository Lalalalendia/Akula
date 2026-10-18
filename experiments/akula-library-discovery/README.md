# Akula Library: Local UDP discovery lab (research only)

This is a **standalone, non-production network experiment**. It does not modify the Akula application or its Sync Core and does not open externally reachable ports. Only synthetic book IDs, terms and source advertisements are used.

## What is really executed

Each index node, central catalog, and synthetic file source is a **separate OS process** with its own UDP socket on **127.0.0.1**. The requester sends actual JSON UDP datagrams and receives OS socket responses. Timings come from the wall clock around those calls; application payload bytes are counted as encoded datagrams. Approximate IPv4/UDP overhead is reported separately (28 B/datagram); no packet capture is performed.

Discovery modes:

- `server`: one of two independent index services. It stores metadata but not book bytes.
- `dht`: simplified sequential (alpha=1) XOR-distance Kademlia-*inspired* search, then paged lookups of keyword->BookID lists at pre-published replicas. **Not libp2p Kademlia.**
- `rwi`: shard-owner roster with direct lookups of paged keyword->BookID lists at 3 volunteer index nodes. **Not YaCy or its wire format.** Roster creation/distribution/verification is assumed, not implemented.
- `hybrid`: prefer central service, fall back to RWI. An in-memory 2-second circuit breaker skips recently unreachable central endpoints.

Book source testing:

- File source port is killed and restarted at a newly bound UDP port.
- Old provider lease remains visible until the owner republishes a replacement; `PING` to the old port fails.
- A short 200-ms provider lease expires and disappears from lookup.
- Both central catalog sockets can be killed, and then one volunteer index socket can be killed; remaining replicas are queried.

## Run

Requires Python 3.10+ with only the standard library:

```bash
cd experiments/akula-library-discovery
python3 -m unittest discover -v
python3 net_lab.py --nodes 3 6 10 --queries 24 --seed 20261018 --output results.json --markdown results.md
```

You can run the same files extracted from the ZIP without placing them in the Akula repository.

## Limits and responsible interpretation

- **Not** an experiment on two computers, public Internet, NAT/CGNAT, mobile background processes, BLE, iroh, QUIC, TCP or TLS.
- **Not** a secure protocol. The synthetic UDP frame has no signatures, encryption, authorization, replay defense or identity proof. NEVER bind this sample to a public interface or send private material through it.
- **Not** a full distributed lifecycle: peer lists are preconfigured; index replication/publication uses known replica addresses; indexes live in RAM and vanish when the index process dies.
- Real data packets traverse loopback, but the IPv4/UDP overhead estimate ignores Ethernet, QUIC/TLS and physical retransmission costs.
- Timeouts of 20-40 ms were chosen for LOCAL testing. Wide-area/mobile networks need different timeouts and adaptive failure estimation.
- Synthetic titles do not model Unicode/morphology, book editions, ranking or censorship/spam attacks. No content files are downloaded.
- Requiring 100% retrieval after loss of one replica is reasonable for this very small, prefilled setup only. The results must not be projected to 1000 devices.

## Recommended follow-up

1. Port the catalog/provider messages to a Rust test harness using the existing Akula STAYA/WOW signed envelope, *without changing Sync Core*.
2. Run on two or three actual hosts on different ISPs and record NAT type, endpoints, real RTT, relay traffic and restart behavior.
3. Add durable signed provider leases and book-card provenance, replay rejection, per-device/group/global query budgets.
4. Only then test large index counts and media transfers with BLAKE3-checked chunks and resumable downloads.

Prior synthetic 10/100/1000-node simulation is archived separately as `Akula_Library_Search_Simulation_20261018.zip` in the project's ChatGPT Library.

Notion project: https://app.notion.com/p/3fd32a84beec81a7af7cf0d1e32f3947
