/* Incremental SHA-256 for bounded-memory browser downloads. No external runtime. */
(function (global) {
  'use strict';
  const K = new Uint32Array([
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
  ]);
  function rotateRight(x, n) { return (x >>> n) | (x << (32 - n)); }
  class IncrementalSha256 {
    constructor() {
      this.state = new Uint32Array([0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
        0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19]);
      this.block = new Uint8Array(64);
      this.words = new Uint32Array(64);
      this.blockLength = 0;
      this.totalBytes = 0n;
      this.finished = false;
    }
    _compress(bytes, offset) {
      const w = this.words;
      for (let i=0; i<16; i++) {
        const j = offset + i * 4;
        w[i] = ((bytes[j] << 24) | (bytes[j+1] << 16) | (bytes[j+2] << 8) | bytes[j+3]) >>> 0;
      }
      for (let i=16; i<64; i++) {
        const x=w[i-15], y=w[i-2];
        const s0=rotateRight(x,7)^rotateRight(x,18)^(x>>>3);
        const s1=rotateRight(y,17)^rotateRight(y,19)^(y>>>10);
        w[i]=(w[i-16]+s0+w[i-7]+s1)>>>0;
      }
      let [a,b,c,d,e,f,g,h]=this.state;
      for (let i=0; i<64; i++) {
        const s1=rotateRight(e,6)^rotateRight(e,11)^rotateRight(e,25);
        const choose=(e&f)^(~e&g);
        const t1=(h+s1+choose+K[i]+w[i])>>>0;
        const s0=rotateRight(a,2)^rotateRight(a,13)^rotateRight(a,22);
        const majority=(a&b)^(a&c)^(b&c);
        const t2=(s0+majority)>>>0;
        h=g;g=f;f=e;e=(d+t1)>>>0;d=c;c=b;b=a;a=(t1+t2)>>>0;
      }
      const v=[a,b,c,d,e,f,g,h];
      for (let i=0; i<8; i++) this.state[i]=(this.state[i]+v[i])>>>0;
    }
    update(data) {
      if (this.finished) throw new Error('SHA-256 already finalized');
      if (!(data instanceof Uint8Array)) throw new TypeError('Expected Uint8Array');
      const length=data.length;
      this.totalBytes+=BigInt(length);
      let pos=0;
      if (this.blockLength) {
        const take=Math.min(64-this.blockLength,length);
        this.block.set(data.subarray(0,take),this.blockLength);
        this.blockLength+=take;pos+=take;
        if (this.blockLength===64) {this._compress(this.block,0);this.blockLength=0;}
      }
      while (pos+64<=length) { this._compress(data,pos);pos+=64; }
      if (pos<length) { this.block.set(data.subarray(pos),0);this.blockLength=length-pos; }
      return this;
    }
    digestHex() {
      if (this.finished) throw new Error('SHA-256 already finalized');
      this.finished=true;
      const bitLength=this.totalBytes*8n;
      if (bitLength>(1n<<64n)-1n) throw new RangeError('SHA-256 input too long');
      let n=this.blockLength;
      this.block[n++]=0x80;
      if (n>56) {this.block.fill(0,n);this._compress(this.block,0);n=0;}
      this.block.fill(0,n,56);
      const view=new DataView(this.block.buffer);
      view.setUint32(56,Number((bitLength>>32n)&0xffffffffn),false);
      view.setUint32(60,Number(bitLength&0xffffffffn),false);
      this._compress(this.block,0);
      return Array.from(this.state,x=>x.toString(16).padStart(8,'0')).join('');
    }
  }
  global.IncrementalSha256=IncrementalSha256;
  if (typeof module !== 'undefined' && module.exports) module.exports={ IncrementalSha256 };
})(typeof globalThis !== 'undefined' ? globalThis : this);
