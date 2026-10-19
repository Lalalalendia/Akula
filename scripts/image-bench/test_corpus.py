import unittest
import corpus

def row(image_id='1019', digest='a'*64, size=1024, origin=None):
    folder='1000-lilith-photos-general';path=f'{folder}/{image_id}_sample_16x16.jpg'
    return [image_id,'test',folder,path,'16','16','jpg',str(size),str(size),digest,origin or corpus.RAW_ROOT+path,'','']

def txt(*rows):
    header=['id','split','content_class','path','width','height','format','bytes_manifest','bytes_actual','sha256','raw_url','png_v3_sdr_url','png_v3_hdr_url']
    return '\t'.join(header)+'\n'+'\n'.join('\t'.join(r) for r in rows)

class Validate(unittest.TestCase):
    def test_valid(self): self.assertEqual(corpus.validate_manifest(txt(row()),('1019',))[0]['sha256'],'a'*64)
    def test_nonallowlisted_origin(self):
        with self.assertRaises(ValueError): corpus.validate_manifest(txt(row(origin='https://example.org/test.jpg')),('1019',))
    def test_duplicate(self):
        with self.assertRaises(ValueError): corpus.validate_manifest(txt(row(),row()),('1019',))
    def test_bad_hash_and_size(self):
        for kw in ({'digest':'wrong'},{'size':33_000_000}):
            with self.subTest(kw=kw),self.assertRaises(ValueError): corpus.validate_manifest(txt(row(**kw)),('1019',))
    def test_missing_id(self):
        with self.assertRaises(ValueError): corpus.validate_manifest(txt(row()),('1019','7007'))

if __name__=='__main__': unittest.main()
