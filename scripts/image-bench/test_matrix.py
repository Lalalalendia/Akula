import unittest
import check_corpus

class Acceptance(unittest.TestCase):
    cases=[{"id":"1019","format":"jpg"},{"id":"5007","format":"png"}]
    @staticmethod
    def entry(id,fmt,mode,trial):
        return {"id":id,"format":fmt,"mode":mode,"attempt":str(trial),"status":"ok","elapsed_sec":"0.01","maxrss_kib":"8000"}
    def matrix(self):
        return [self.entry(id,fmt,m,trial) for id,fmt in [("1019","jpg"),("5007","png")]
                for m in check_corpus.expected_modes(fmt) for trial in range(1,4)]
    def test_complete(self):
        self.assertEqual(check_corpus.verify(self.cases,self.matrix(),[],3),[])
    def test_missing_decoder_is_failure(self):
        ms=[m for m in self.matrix() if not (m["id"]=="1019" and m["mode"]=="turbo")]
        errors=check_corpus.verify(self.cases,ms,[],3)
        self.assertTrue(any("missing required" in e for e in errors))
    def test_failed_decoder_must_not_be_green(self):
        errs=check_corpus.verify(self.cases,self.matrix(),[{"id":"1019","mode":"turbo","attempt":"2","reason":"exit_3"}],3)
        self.assertTrue(any("recorded failure" in e for e in errs))
    def test_empty_or_invalid_memory_is_rejected(self):
        ms=self.matrix();ms[0]["maxrss_kib"]="0"
        errs=check_corpus.verify(self.cases,ms,[],3)
        self.assertTrue(any("invalid time/RSS" in e for e in errs))

if __name__=="__main__":unittest.main()
