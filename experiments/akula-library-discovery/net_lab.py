#!/usr/bin/env python3
"""Akula Library: actual UDP socket/process discovery experiment.

Not a QUIC/iroh/Kademlia implementation; educational controlled network lab.
Each node is a separate OS process, bound to an actual 127.0.0.1 UDP socket.
No private books, cryptography, NAT traversal or external network are exercised.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import multiprocessing as mp
import random
import socket
import statistics
import time
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

FRAME_MAX = 1200  # avoid IP fragmentation in basic Ethernet MTU environments
PAGE = 12
LOOKUP_MAX = 15
REPLICA_COUNT = 3
RPC_TIMEOUT = 0.040


def hashed(s: str) -> int:
    return int.from_bytes(hashlib.sha256(s.encode('utf-8')).digest()[:8], 'big')


def coded(message: dict) -> bytes:
    raw = json.dumps(message, ensure_ascii=True, separators=(',', ':'), sort_keys=True).encode('utf-8')
    if len(raw) > FRAME_MAX:
        raise ValueError(f'Frame too large: {len(raw)}')
    return raw


def percentile(xs: list[float], p: int) -> float:
    if not xs:
        return 0.0
    values = sorted(xs)
    return round(values[max(0, math.ceil(len(values)*p/100)-1)], 3)


@dataclass
class Meter:
    request_bytes: int = 0
    reply_bytes: int = 0
    requests: int = 0
    replies: int = 0
    failures: int = 0

    def add_request(self, size: int) -> None:
        self.request_bytes += size
        self.requests += 1

    def add_response(self, size: int) -> None:
        self.reply_bytes += size
        self.replies += 1

    @property
    def wire_payload_bytes(self) -> int:
        return self.request_bytes+self.reply_bytes

    @property
    def estimated_ipv4_udp_bytes(self) -> int:
        return self.wire_payload_bytes + 28 * (self.requests + self.replies)


class Rpc:
    def __init__(self, meter: Meter | None = None, timeout: float = RPC_TIMEOUT):
        self.meter = meter or Meter()
        self.timeout = timeout
        self.serial = 0

    def ask(self, port: int, op: str, **fields) -> dict | None:
        self.serial += 1
        req = coded({'op': op, 'rid': self.serial, **fields})
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(('127.0.0.1', 0))
            sock.settimeout(self.timeout)
            self.meter.add_request(len(req))
            sock.sendto(req, ('127.0.0.1', port))
            try:
                while True:
                    data, _ = sock.recvfrom(4096)
                    if len(data) > FRAME_MAX:
                        continue
                    response = json.loads(data)
                    if response.get('rid') == self.serial:
                        self.meter.add_response(len(data))
                        return response
            except (socket.timeout, OSError, json.JSONDecodeError):
                self.meter.failures += 1
                return None


def worker(kind: str, node_id: int, control) -> None:
    """One remote-index or book-source process. State lasts only while alive."""
    index: dict[str, dict[str, tuple[dict, float]]] = defaultdict(dict)
    cards: dict[str, tuple[str, str]] = {}
    known: list[dict] = []
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(('127.0.0.1', 0))
        sock.settimeout(0.1)
        port = sock.getsockname()[1]
        control.send({'id': node_id, 'port': port, 'kind': kind})
        control.close()
        while True:
            try:
                data, addr = sock.recvfrom(4096)
            except socket.timeout:
                continue
            if len(data) > FRAME_MAX:
                continue
            try:
                m = json.loads(data)
                rid = m.get('rid', 0)
                op = m.get('op')
                if op == 'HALT':
                    return
                answer: dict = {'rid': rid, 'ok': True}
                if op == 'PING':
                    answer['node'] = node_id
                elif op == 'CONFIG':
                    if kind != 'index':
                        answer['ok'] = False
                    else:
                        known = m.get('neighbors', [])[:64]
                elif op == 'NEAR':
                    if kind != 'index':
                        answer['ok'] = False
                    else:
                        key = int(m['key'])
                        nearest = sorted(known+[{'id': node_id, 'port':port}],
                                         key=lambda x: int(x['id']) ^ key)[:6]
                        answer['peers'] = nearest
                elif op == 'PUT':
                    if kind != 'index':
                        answer['ok'] = False
                    else:
                        key = str(m['key'])
                        value = m['value']
                        ttl_ms = min(120000, max(50, int(m.get('ttl_ms', 90000))))
                        if len(value) > 5:
                            answer['ok'] = False
                        else:
                            index[key][str(value['id'])] = (value, time.monotonic()+ttl_ms/1000)
                elif op == 'GET':
                    if kind != 'index':
                        answer['ok'] = False
                    else:
                        key = str(m['key'])
                        records = index.get(key, {})
                        now = time.monotonic()
                        for rid0 in list(records):
                            if records[rid0][1] <= now:
                                del records[rid0]
                        values = [records[k][0] for k in sorted(records)]
                        offset = max(0, int(m.get('after',0)))
                        max_items = min(PAGE, max(1, int(m.get('limit', PAGE))))
                        answer.update({'items': values[offset:offset+max_items],
                                       'next': offset+max_items if offset+max_items<len(values) else None})
                elif op == 'CARD':
                    if kind != 'catalog':
                        answer['ok'] = False
                    else:
                        cards[str(m['id'])] = (str(m['a']), str(m['b']))
                elif op == 'SEARCH':
                    if kind != 'catalog':
                        answer['ok'] = False
                    else:
                        a,b = m['a'],m['b']
                        ids = sorted(k for k,(x,y) in cards.items() if x==a and y==b)
                        after=max(0,int(m.get('after',0)))
                        answer['items'] = ids[after:after+PAGE]
                        answer['next'] = after+PAGE if after+PAGE<len(ids) else None
                else:
                    answer['ok'] = False
                    answer['error'] = 'unknown-op'
                response = coded(answer)
                sock.sendto(response, addr)
            except (ValueError, TypeError, KeyError, OverflowError):
                # A malformed request must not kill a network node.
                continue


class Node:
    def __init__(self, kind: str, node_id: int):
        # Forkserver avoids inheriting host threads when available.
        # Windows has no forkserver, so use spawn there.
        self.ctx = mp.get_context('forkserver' if 'forkserver' in mp.get_all_start_methods() else 'spawn')
        self.parent, self.child = self.ctx.Pipe(duplex=False)
        self.proc = self.ctx.Process(target=worker, args=(kind, node_id, self.child))
        self.proc.start()
        self.child.close()
        self.kind = kind
        self.id = node_id
        try:
            if not self.parent.poll(8):
                raise TimeoutError('node failed to bind socket')
            self.port = self.parent.recv()['port']
            self.parent.close()
        except (TimeoutError, EOFError) as exc:
            self.proc.terminate()
            self.proc.join(2)
            raise RuntimeError('node failed to bind socket') from exc

    def kill(self):
        if self.proc.is_alive():
            self.proc.terminate()  # deliberate non-graceful network loss
        self.proc.join(2)


@dataclass
class Trial:
    ok: bool
    millis: float
    meter: Meter


@dataclass
class Book:
    id: str
    a: str
    b: str


class Lab:
    def __init__(self, n: int, seed: int = 42, books_per_node: int = 12):
        if n < 3 or n > 100:
            raise ValueError('network experiment supports 3..100 index peers')
        r = random.Random(seed+101*n)
        self.nodes = [Node('index', r.getrandbits(64)) for _ in range(n)]
        self.centers = [Node('catalog', r.getrandbits(64)) for _ in range(2)]
        self.source = Node('source', r.getrandbits(64))
        self.books = [Book(f'b{i:04}', f'a{r.randrange(10):02}',f'b{r.randrange(15):02}')
                      for i in range(n*books_per_node)]
        self.bootstraps = self.nodes[:2]
        self.stats = Meter()
        self._server_unhealthy_until: dict[int, float] = {}
        self._configure()
        self._publish()

    def shutdown(self):
        for node in [*self.nodes, *self.centers, self.source]:
            node.kill()

    def _routing(self, src: Node) -> list[dict]:
        # Small bounded XOR buckets. Configured over real UDP RPCs for reproducibility.
        groups: dict[int, list[Node]] = defaultdict(list)
        for p in self.nodes:
            if p is src:
                continue
            distance = src.id ^ p.id
            groups[distance.bit_length()-1].append(p)
        selected = []
        for peers in groups.values():
            selected.extend(sorted(peers, key=lambda x: x.id)[:3])
        return [{'id':p.id,'port':p.port} for p in selected]

    def _configure(self):
        rpc=Rpc(self.stats)
        for node in self.nodes:
            response=rpc.ask(node.port,'CONFIG',neighbors=self._routing(node))
            if not response or not response.get('ok'):
                raise RuntimeError('routing configuration failed')

    def replicas(self, key: str) -> list[Node]:
        target=hashed(key)
        return sorted(self.nodes,key=lambda n:n.id ^ target)[:REPLICA_COUNT]

    def put(self,key:str,value:dict,ttl_ms:int=90000):
        rpc=Rpc(self.stats)
        for node in self.replicas(key):
            response=rpc.ask(node.port,'PUT',key=key,value=value,ttl_ms=ttl_ms)
            if response is None or not response.get('ok'):
                raise RuntimeError(f'publish failed on {node.port}: {key}')

    def _publish(self):
        rpc=Rpc(self.stats)
        for book in self.books:
            for c in self.centers:
                response=rpc.ask(c.port,'CARD',id=book.id,a=book.a,b=book.b)
                if not response or not response.get('ok'):
                    raise RuntimeError('central publish failed')
            for term in (book.a,book.b):
                self.put('kw:'+term,{'id':book.id})
                self.put('rwi:'+term,{'id':book.id})
            self.put('cid:'+book.id, {'id':'source', 'port':self.source.port, 'owner':self.source.id})

    def _walk(self, key: str, rpc: Rpc) -> list[Node]:
        """Iterative XOR peer discovery over actual UDP, alpha=1 simplified test."""
        target = hashed(key)
        known:dict[int,tuple[int,int]]={n.id:(n.port,0) for n in self.bootstraps}
        asked=set()
        for _ in range(LOOKUP_MAX):
            candidates=[(nid,p) for nid,(p,_) in known.items() if nid not in asked]
            if not candidates:
                break
            nid,port=min(candidates, key=lambda t:t[0]^target)
            asked.add(nid)
            response=rpc.ask(port,'NEAR',key=target)
            if response and response.get('ok'):
                for peer in response['peers']:
                    known[int(peer['id'])]=(int(peer['port']),0)
            if len(asked)>=len(self.nodes):
                break
        nearest=sorted(known,key=lambda nid:nid ^ target)
        return [self._find_node(nid) for nid in nearest if self._find_node(nid) is not None]

    def _find_node(self,nid:int) -> Node | None:
        for node in self.nodes:
            if node.id==nid:
                return node
        return None

    def _get_pages(self, node:Node,key:str,rpc:Rpc) -> list[dict]|None:
        after=0
        output=[]
        for _ in range(200):
            response=rpc.ask(node.port,'GET',key=key,after=after,limit=PAGE)
            if not response or not response.get('ok'):
                return None
            output.extend(response['items'])
            if response.get('next') is None:
                return output
            after=response['next']
        return None

    def get(self,key:str,rpc:Rpc,mode:str) -> list[dict]:
        candidates=self.replicas(key) if mode=='rwi' else self._walk(key,rpc)
        actual_replica_ids={n.id for n in self.replicas(key)}
        for node in candidates:
            if node.id not in actual_replica_ids:
                continue
            records=self._get_pages(node,key,rpc)
            if records is not None:
                return records
        return []

    def query(self,book:Book, mode:str,timeout:float=RPC_TIMEOUT) -> Trial:
        meter=Meter()
        rpc=Rpc(meter,timeout)
        started=time.perf_counter()
        ids=[]
        if mode in ('server','hybrid'):
            for center in self.centers:
                if mode == 'hybrid' and time.monotonic() < self._server_unhealthy_until.get(center.port, 0):
                    continue
                after=0
                for _ in range(200):
                    r=rpc.ask(center.port,'SEARCH',a=book.a,b=book.b,after=after)
                    if not r or not r.get('ok'):
                        if mode == 'hybrid':
                            self._server_unhealthy_until[center.port] = time.monotonic()+2.0
                        break
                    if mode == 'hybrid':
                        self._server_unhealthy_until.pop(center.port,None)
                    ids.extend(r.get('items',[]))
                    if r.get('next') is None or book.id in ids:
                        break
                    after=int(r['next'])
                if ids:
                    break
            if not ids and mode=='server':
                return Trial(False,(time.perf_counter()-started)*1000,meter)
        if not ids and mode in ('hybrid','rwi','dht'):
            prefix='rwi:' if mode in ('hybrid','rwi') else 'kw:'
            x=self.get(prefix+book.a,rpc,'rwi' if mode in ('hybrid','rwi') else 'dht')
            y=self.get(prefix+book.b,rpc,'rwi' if mode in ('hybrid','rwi') else 'dht')
            ids=list({v['id'] for v in x}&{v['id'] for v in y})
        return Trial(book.id in ids,(time.perf_counter()-started)*1000,meter)

    def provider(self,book:Book) -> list[dict]:
        return self.get('cid:'+book.id,Rpc(timeout=.030),'dht')

    def source_rebind(self,book:Book):
        oldport=self.source.port
        self.source.kill()
        self.source=Node('source',self.source.id)
        assert self.source.port!=oldport, 'the OS unexpectedly recycled the same port'
        return oldport,self.source.port


def summarize(rows: list[Trial]) -> dict:
    return {
        'found_pct':round(100*sum(t.ok for t in rows)/len(rows),2),
        'p50_ms':percentile([t.millis for t in rows],50),
        'p95_ms':percentile([t.millis for t in rows],95),
        'avg_udp_payload_bytes':round(sum(t.meter.wire_payload_bytes for t in rows)/len(rows),1),
        'avg_ipv4_udp_bytes_est':round(sum(t.meter.estimated_ipv4_udp_bytes for t in rows)/len(rows),1),
        'avg_rpc':round(sum(t.meter.requests for t in rows)/len(rows),2),
        'avg_timeouts':round(sum(t.meter.failures for t in rows)/len(rows),2),
    }


def bench(n:int,seed:int, queries:int) -> dict:
    started_bench=time.perf_counter()
    lab=Lab(n,seed)
    print('ready:',n,'seconds:',round(time.perf_counter()-started_bench,2),flush=True)
    try:
        r=random.Random(seed*2+n)
        sample=[r.choice(lab.books) for _ in range(queries)]
        normal={m:summarize([lab.query(b,m) for b in sample]) for m in ('server','dht','rwi','hybrid')}
        print('searched normal:',n,'seconds:',round(time.perf_counter()-started_bench,2),flush=True)
        source_book=lab.books[-1]
        provider_before=lab.provider(source_book)
        assert provider_before and provider_before[0]['port']==lab.source.port
        old,new=lab.source_rebind(source_book)
        probe=Rpc(timeout=.020)
        old_dead=probe.ask(old,'PING') is None
        stale_before=lab.provider(source_book)
        lab.put('cid:'+source_book.id,{'id':'source','port':new,'owner':lab.source.id},ttl_ms=90000)
        rebinding=lab.provider(source_book)
        new_live=probe.ask(new,'PING') is not None
        # Short lease, do not confuse a stale provider record with deletion of a book.
        lab.put('cid:lease-expiry-check',{'id':'short','port':new},ttl_ms=200)
        initially_present=bool(lab.get('cid:lease-expiry-check',Rpc(),'rwi'))
        time.sleep(.28)
        expired=not bool(lab.get('cid:lease-expiry-check',Rpc(),'rwi'))
        # Cold loss of real listening sockets: both center ports become unreachable.
        for center in lab.centers:
            center.kill()
        print('source rebind and outage:',n,'seconds:',round(time.perf_counter()-started_bench,2),flush=True)
        outage={m:summarize([lab.query(b,m,timeout=.020) for b in sample[:min(queries,20)]])
                for m in ('server','dht','rwi','hybrid')}
        print('outage searches:',n,'seconds:',round(time.perf_counter()-started_bench,2),flush=True)
        # A single indexer's process abruptly dies. Remaining replicas should serve.
        lab.nodes[0].kill()
        partial={m:summarize([lab.query(b,m,timeout=.020) for b in sample[:min(queries,20)]])
                 for m in ('dht','rwi','hybrid')}
        print('partial searches:',n,'seconds:',round(time.perf_counter()-started_bench,2),flush=True)
        return {'nodes':n,'books':len(lab.books),'queries':queries,'normal':normal,
                'centers_offline':outage,'centers_and_one_index_offline':partial,
                'source_rebind':{'old_port':old,'new_port':new,
                  'old_port_dead':old_dead,'old_advertisement_visible_before_refresh':bool(stale_before and stale_before[0]['port']==old),
                  'new_advertisement_visible':bool(rebinding and rebinding[0]['port']==new),
                  'new_port_live':new_live},
                'lease_expiry':{'present_initially':initially_present,'expired_after_280ms':expired},
                'publication_udp_payload_bytes':lab.stats.wire_payload_bytes,
                'publication_packets':lab.stats.requests+lab.stats.replies}
    finally:
        lab.shutdown()


def markdown(results:list[dict]) -> str:
    lines=['# Akula Library — реальный UDP стенд на одной машине',
           '', '**Граница результатов:** отдельные процессы и настоящие UDP-сокеты ОС, но только 127.0.0.1. Это **не** реальные телефоны, QUIC, iroh, WAN, NAT или Bluetooth. Все книги и сообщения искусственные. Неэквивалентно реальному libp2p Kademlia: в опыте упрощённый последовательный поиск XOR и предварительно заданные соседи.',
           '', 'Время взято из `time.perf_counter` (фактическая длительность UDP вызовов и ожидания таймаута), а не искусственной функции RTT. Объём полезной нагрузки измерен по числу отправленных/полученных байтов JSON, сетевой IPv4+UDP заголовок дан отдельной **оценкой** 28 байт/пакет, не сетевым захватом.',
           '', '## Поиск: p50 / p95 на localhost',
           '', '| Узлов индекса | Состояние | Алгоритм | Успех | p50 мс | p95 мс | Трафик, Б/запрос, оценка IPv4+UDP |',
           '|---:|---|---|---:|---:|---:|---:|']
    for r in results:
        for state,label in [('normal','Норма'),('centers_offline','2 сервера выключены'),('centers_and_one_index_offline','2 сервера и 1 индексатор выключены')]:
            for name,stat in r[state].items():
                lines.append(f"| {r['nodes']} | {label} | {name} | {stat['found_pct']}% | {stat['p50_ms']} | {stat['p95_ms']} | {stat['avg_ipv4_udp_bytes_est']} |")
    lines += ['', '## Переподключение источника и истечение объявлений', '',
              '| Узлов | Старый порт не отвечает | Старое объявление осталось до публикации | Новый адрес найден | Новый адрес отвечает | Короткая запись истекла |',
              '|---:|---|---|---|---|---|']
    for r in results:
        z=r['source_rebind'];e=r['lease_expiry']
        lines.append(f"| {r['nodes']} | {z['old_port_dead']} | {z['old_advertisement_visible_before_refresh']} | {z['new_advertisement_visible']} | {z['new_port_live']} | {e['present_initially'] and e['expired_after_280ms']} |")
    lines += ['', '## Чего этот стенд не доказывает', '',
              '- Нет внешних IP, операторских NAT/CGNAT, отдельной Wi-Fi/сотовой сети, радио или усыпления Android/iOS.',
              '- Нет сквозной криптографии, подлинности издателя, ограничений прав, книги не скачиваются: только поиск карточек и адресов.',
              '- Нет устойчивого дискового индекса/восстановления после потери индексатора; при отказе нескольких реплик данные могут исчезнуть.',
              '- Нет сетевого TLS/QUIC, измеренной нагрузки батареи и честной передачи частей с ограничением трафика.',
              '- Публикация DHT/RWI в опыте осуществлена по заранее известным адресам реплик; bootstrap/обновление подписанного списка узлов ещё не разработаны.',
              '- Результаты не включают сложную морфологию книг, опечатки, ложные объявления, перегрузку популярных слов или крупные каталоги.',
              '', '## Далее', '',
              '1. Разнести процессы на два-три реальных хоста с разными провайдерами, зафиксировать NAT и реальный RTT.',
              '2. Написать защищённый Rust-iroh транспорт в отдельном стенде, интегрировать проверяемые BookCard и ProviderLease.',
              '3. Проверить мобильный Android/iOS фон, смену Wi-Fi/сотовой сети, имитацию массовых отказов и подмену объявлений.',
              '4. Затем нагрузить 100/1000 узлов **в реальной сетевой инфраструктуре**, не выдавая расчётную модель за измерение.']
    return '\n'.join(lines)+'\n'


def acceptance(results:list[dict]) -> None:
    for r in results:
        n=r['nodes']
        for name in ('server','dht','rwi','hybrid'):
            if r['normal'][name]['found_pct'] != 100:
                raise AssertionError(f'normal search incomplete: {n} {name}')
        if r['centers_offline']['server']['found_pct'] != 0:
            raise AssertionError(f'central search succeeded after stop: {n}')
        for name in ('dht','rwi','hybrid'):
            for scenario in ('centers_offline','centers_and_one_index_offline'):
                if r[scenario][name]['found_pct'] != 100:
                    raise AssertionError(f'fallback not complete: {n} {scenario} {name}')
        rebinding=r['source_rebind']
        expiry=r['lease_expiry']
        if not all(rebinding[key] for key in ('old_port_dead','old_advertisement_visible_before_refresh',
                   'new_advertisement_visible','new_port_live')):
            raise AssertionError(f'source rebind failed: {n}')
        if not expiry['present_initially'] or not expiry['expired_after_280ms']:
            raise AssertionError(f'lease TTL failed: {n}')


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument('--nodes',nargs='+',type=int,default=[3,6,10])
    ap.add_argument('--queries',type=int,default=24)
    ap.add_argument('--seed',type=int,default=20261018)
    ap.add_argument('--output',default='results.json')
    ap.add_argument('--markdown',default='results.md')
    args=ap.parse_args()
    all_results=[]
    for n in args.nodes:
        record=bench(n,args.seed,args.queries)
        all_results.append(record)
        print('nodes:',n,'normal', {k:(v['found_pct'],v['p50_ms']) for k,v in record['normal'].items()},flush=True)
    acceptance(all_results)
    Path(args.output).write_text(json.dumps({'kind':'localhost_udp_real_sockets',
          'note':'Not WAN or QUIC; latency real loopback, IP overhead estimated',
          'results':all_results},ensure_ascii=False,indent=2),encoding='utf-8')
    Path(args.markdown).write_text(markdown(all_results),encoding='utf-8')


if __name__=='__main__':
    main()
