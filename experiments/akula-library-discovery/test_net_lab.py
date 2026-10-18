"""Network tests use real UDP sockets and separate OS processes."""
import unittest
from net_lab import Lab, Rpc


class UDPDiscoveryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.lab = Lab(3,seed=123, books_per_node=3)

    @classmethod
    def tearDownClass(cls):
        cls.lab.shutdown()

    def test_central_rwi_and_dht_return_same_card(self):
        book=self.lab.books[0]
        for mode in ('server','rwi','dht','hybrid'):
            with self.subTest(mode=mode):
                self.assertTrue(self.lab.query(book,mode).ok)

    def test_provider_announced(self):
        book=self.lab.books[1]
        providers=self.lab.provider(book)
        self.assertEqual(providers[0]['port'],self.lab.source.port)
        self.assertIsNotNone(Rpc().ask(providers[0]['port'],'PING'))

    def test_known_key_vs_unknown_key(self):
        self.assertEqual(self.lab.get('cid:nonexistent-1001',Rpc(),'dht'),[])

    def test_private_card_not_in_public_network(self):
        self.assertEqual(self.lab.get('kw:private-only-title',Rpc(),'dht'),[])

    def test_malformed_packet_does_not_crash(self):
        import socket
        target=self.lab.nodes[0]
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:
            s.sendto(b'not_json',( '127.0.0.1',target.port))
            s.sendto(b'A'*1300,('127.0.0.1',target.port))
        self.assertIsNotNone(Rpc().ask(target.port,'PING'))

# The tests below are separate functions because the five basic unittest cases
# above share a pre-published live lab. Each test creates its own isolated lab.
class UDPFailureTests(unittest.TestCase):
    def test_both_central_indexers_can_die(self):
        lab=Lab(3,seed=137,books_per_node=2)
        try:
            b=lab.books[0]
            for c in lab.centers:
                c.kill()
            self.assertFalse(lab.query(b,'server',timeout=.010).ok)
            self.assertTrue(lab.query(b,'dht',timeout=.010).ok)
            self.assertTrue(lab.query(b,'rwi',timeout=.010).ok)
            self.assertTrue(lab.query(b,'hybrid',timeout=.010).ok)
        finally:
            lab.shutdown()

    def test_source_rebind_and_lease_expiry(self):
        import time
        lab=Lab(3,seed=113,books_per_node=2)
        try:
            b=lab.books[0]
            old,new=lab.source_rebind(b)
            self.assertIsNone(Rpc(timeout=.015).ask(old,'PING'))
            self.assertEqual(lab.provider(b)[0]['port'],old)  # stale, not live
            lab.put('cid:'+b.id,{'id':'source','port':new,'owner':lab.source.id})
            self.assertEqual(lab.provider(b)[0]['port'],new)
            self.assertIsNotNone(Rpc().ask(new,'PING'))
            lab.put('cid:ttl-test',{'id':'source','port':new},ttl_ms=200)
            self.assertTrue(lab.get('cid:ttl-test',Rpc(),'rwi'))
            time.sleep(.29)
            self.assertFalse(lab.get('cid:ttl-test',Rpc(),'rwi'))
        finally:
            lab.shutdown()

    def test_one_index_replica_failure(self):
        lab=Lab(3,seed=101,books_per_node=2)
        try:
            b=lab.books[-1]
            lab.nodes[0].kill()
            self.assertTrue(lab.query(b,'dht',timeout=.010).ok)
            self.assertTrue(lab.query(b,'rwi',timeout=.010).ok)
        finally:
            lab.shutdown()

    def test_message_limit(self):
        from net_lab import coded, FRAME_MAX
        with self.assertRaises(ValueError):
            coded({'long':'X'*(FRAME_MAX+1)})

    def test_hybrid_skips_recently_dead_servers(self):
        lab=Lab(3,seed=167,books_per_node=2)
        try:
            for center in lab.centers:
                center.kill()
            b=lab.books[0]
            first=lab.query(b,'hybrid',timeout=.010)
            second=lab.query(b,'hybrid',timeout=.010)
            self.assertTrue(first.ok and second.ok)
            self.assertEqual(first.meter.failures,2)
            self.assertEqual(second.meter.failures,0)
            self.assertLess(second.millis,first.millis)
        finally:
            lab.shutdown()


if __name__ == '__main__':
    unittest.main()
