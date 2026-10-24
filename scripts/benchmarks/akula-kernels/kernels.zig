//! Zig 0.17 standalone equivalents of Akula half/dilate/splice/exact matcher.
const std=@import("std");
fn min(a:usize,b:usize) usize {return @min(a,b);}
fn load32(p:[*]const u8) usize {
    return @as(usize,p[0])|(@as(usize,p[1])<<8)|(@as(usize,p[2])<<16)|(@as(usize,p[3])<<24);
}
fn store32(p:[*]u8,n:usize) void {
    p[0]=@intCast(n&255);p[1]=@intCast((n>>8)&255);
    p[2]=@intCast((n>>16)&255);p[3]=@intCast((n>>24)&255);
}
export fn ak_mipmap(src:[*]const u8,dst:[*]u8,w:usize,h:usize) u32 {
    if(w==0 or h==0 or (w==1 and h==1))return 0;
    const dw=(w+1)/2;const dh=(h+1)/2;
    var translucent=false;
    var y:usize=0;
    while(y<dh):(y+=1){
        const y0=2*y;const y1=min(y0+1,h-1);
        var x:usize=0;
        while(x<dw):(x+=1){
            const x0=2*x;const x1=min(x0+1,w-1);
            const a=4*(y0*w+x0);const b=4*(y0*w+x1);
            const c=4*(y1*w+x0);const d=4*(y1*w+x1);
            var ch:usize=0;
            while(ch<4):(ch+=1){
                const sum:u16=@as(u16,src[a+ch])+@as(u16,src[b+ch])+@as(u16,src[c+ch])+@as(u16,src[d+ch]);
                const v:u8=@intCast((sum+2)/4);
                dst[4*(y*dw+x)+ch]=v;
                if(ch==3 and v!=255) translucent=true;
            }
        }
    }
    return if(translucent)2 else 1;
}
export fn ak_dilate(mask:[*]const u8,tmp:[*]u8,out:[*]u8,w:usize,h:usize) void {
    if(w==0 or h==0)return;
    var y:usize=0;
    while(y<h):(y+=1){
        const row=y*w;
        var x:usize=0;
        while(x<w):(x+=1){
            tmp[row+x]=@max(mask[row+x],@max(mask[row+(if(x==0)@as(usize,0) else x-1)],mask[row+min(x+1,w-1)]));
        }
    }
    y=0;
    while(y<h):(y+=1){
        const prev=(if(y==0)@as(usize,0) else y-1)*w;
        const cur=y*w;const next=min(y+1,h-1)*w;
        var x:usize=0;
        while(x<w):(x+=1){
            out[cur+x]=@max(tmp[prev+x],@max(tmp[cur+x],tmp[next+x]));
        }
    }
}
export fn ak_splice(base:[*]const u8,bn:usize,target:[*]const u8,tn:usize,patch:[*]u8) usize {
    const common=min(bn,tn);
    var prefix:usize=0;
    while(prefix<common and base[prefix]==target[prefix]):(prefix+=1){}
    var suffix:usize=0;
    while(suffix<common-prefix and base[bn-1-suffix]==target[tn-1-suffix]):(suffix+=1){}
    const removed=bn-prefix-suffix;
    const inserted=tn-prefix-suffix;
    store32(patch,prefix);store32(patch+4,removed);store32(patch+8,inserted);
    var i:usize=0;
    while(i<inserted):(i+=1)patch[12+i]=target[prefix+i];
    return 12+inserted;
}
export fn ak_apply(base:[*]const u8,bn:usize,patch:[*]const u8,plen:usize,dst:[*]u8) usize {
    if(plen<12)return 0;
    const prefix=load32(patch);const removed=load32(patch+4);const inserted=load32(patch+8);
    if(prefix>bn or removed>bn-prefix or inserted!=plen-12)return 0;
    var i:usize=0;
    while(i<prefix):(i+=1)dst[i]=base[i];
    i=0;
    while(i<inserted):(i+=1)dst[prefix+i]=patch[12+i];
    i=0;
    while(i<bn-prefix-removed):(i+=1)dst[prefix+inserted+i]=base[prefix+removed+i];
    return bn-removed+inserted;
}
export fn ak_search(text:[*]const u8,n:usize,needles:[*]const u8,
                    offsets:[*]const u32,count:usize,hits:[*]u32,capacity:usize) usize {
    var found:usize=0;var i:usize=0;
    while(i<n){
        var matched:usize=0;
        var k:usize=0;
        while(k<count):(k+=1){
            const start:usize=@intCast(offsets[k]);
            const end:usize=@intCast(offsets[k+1]);
            const len=end-start;
            if(len>0 and len<=n-i and text[i]==needles[start]){
                var j:usize=0;
                while(j<len and text[i+j]==needles[start+j]):(j+=1){}
                if(j==len){matched=len;break;}
            }
        }
        if(matched>0){
            if(found>=capacity)return 0;
            hits[2*found]=@intCast(i);
            hits[2*found+1]=@intCast(i+matched);
            found+=1;i+=matched;
        }else{
            const b=text[i];
            i+=if(b<128)@as(usize,1) else if(b<224)@as(usize,2) else if(b<240)@as(usize,3) else @as(usize,4);
        }
    }
    return found;
}
