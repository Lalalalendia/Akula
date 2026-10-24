//! Isolated ports of Akula's half, dilate, splice/apply and
//! Fold::Exact/Substring matcher. Fixed valid nonaliasing C ABI buffers.
//! NOT the installed application; code can be independently compared with main.
use std::{cmp::min, ptr, slice};

#[no_mangle]
pub unsafe extern "C" fn ak_mipmap(src:*const u8,dst:*mut u8,w:usize,h:usize)->u32{
    if w==0 || h==0 || (w==1 && h==1){return 0;}
    let source=slice::from_raw_parts(src,w*h*4);
    let output=slice::from_raw_parts_mut(dst,w.div_ceil(2)*h.div_ceil(2)*4);
    let (dw,dh)=(w.div_ceil(2),h.div_ceil(2));
    let mut translucent=false;
    for y in 0..dh {
        let y0=2*y;let y1=min(y0+1,h-1);
        for x in 0..dw {
            let x0=2*x;let x1=min(x0+1,w-1);
            let offsets=[(y0*w+x0)*4,(y0*w+x1)*4,(y1*w+x0)*4,(y1*w+x1)*4];
            for ch in 0..4 {
                let sum:u16=offsets.iter().map(|&i|u16::from(source[i+ch])).sum();
                let v=((sum+2)/4) as u8;
                output[(y*dw+x)*4+ch]=v;
                if ch==3 && v!=255 { translucent=true; }
            }
        }
    }
    if translucent{2}else{1}
}
#[no_mangle]
pub unsafe extern "C" fn ak_dilate(mask:*const u8, horizontal:*mut u8,
                                      out:*mut u8, w:usize,h:usize){
    if w==0 || h==0{return;}
    let original=slice::from_raw_parts(mask,w*h);
    let tmp=slice::from_raw_parts_mut(horizontal,w*h);
    let result=slice::from_raw_parts_mut(out,w*h);
    for y in 0..h {
        let row=y*w;
        for x in 0..w {
            tmp[row+x]=original[row+x].max(original[row+x.saturating_sub(1)])
                .max(original[row+min(x+1,w-1)]);
        }
    }
    for y in 0..h {
        let p=y.saturating_sub(1)*w;let cur=y*w;let n=min(y+1,h-1)*w;
        for x in 0..w{result[cur+x]=tmp[p+x].max(tmp[cur+x]).max(tmp[n+x]);}
    }
}
#[no_mangle]
pub unsafe extern "C" fn ak_splice(base:*const u8,bn:usize,target:*const u8,
                                    tn:usize,patch:*mut u8)->usize{
    let old=slice::from_raw_parts(base,bn);
    let new=slice::from_raw_parts(target,tn);
    let common=min(bn,tn);
    let mut prefix=0;
    while prefix<common && old[prefix]==new[prefix]{prefix+=1;}
    let mut suffix=0;
    while suffix<common-prefix && old[bn-1-suffix]==new[tn-1-suffix]{suffix+=1;}
    let removed=bn-prefix-suffix;
    let inserted=tn-prefix-suffix;
    let result=slice::from_raw_parts_mut(patch,12+tn);
    result[0..4].copy_from_slice(&(prefix as u32).to_le_bytes());
    result[4..8].copy_from_slice(&(removed as u32).to_le_bytes());
    result[8..12].copy_from_slice(&(inserted as u32).to_le_bytes());
    result[12..12+inserted].copy_from_slice(&new[prefix..tn-suffix]);
    12+inserted
}
#[no_mangle]
pub unsafe extern "C" fn ak_apply(base:*const u8,bn:usize,patch:*const u8,
                                   plen:usize,dst:*mut u8)->usize{
    if plen<12{return usize::MAX;}
    let p=slice::from_raw_parts(patch,plen);
    let word=|n:usize|u32::from_le_bytes(p[n..n+4].try_into().unwrap()) as usize;
    let (prefix,removed,inserted)=(word(0),word(4),word(8));
    if prefix>bn || removed>bn-prefix || inserted!=plen-12{return usize::MAX;}
    let result_len=bn-removed+inserted;
    let output=slice::from_raw_parts_mut(dst,result_len);
    let old=slice::from_raw_parts(base,bn);
    output[..prefix].copy_from_slice(&old[..prefix]);
    output[prefix..prefix+inserted].copy_from_slice(&p[12..]);
    output[prefix+inserted..].copy_from_slice(&old[prefix+removed..]);
    result_len
}
#[no_mangle]
pub unsafe extern "C" fn ak_search(text:*const u8,n:usize,needles:*const u8,
                                    offsets:*const u32,count:usize,
                                    hits:*mut u32,capacity:usize)->usize{
    let source=slice::from_raw_parts(text,n);
    let off=slice::from_raw_parts(offsets,count+1);
    let search_bytes=slice::from_raw_parts(needles,off[count] as usize);
    let matches=slice::from_raw_parts_mut(hits,capacity*2);
    let mut found=0;
    let mut i=0;
    while i<n {
        let mut matched=0;
        for k in 0..count {
            let start=off[k] as usize;
            let len=(off[k+1]-off[k]) as usize;
            if len>0 && len<=n-i && source[i..].starts_with(&search_bytes[start..start+len]){
                matched=len;break;
            }
        }
        if matched>0 {
            if found>=capacity {return usize::MAX;}
            matches[2*found]=i as u32;
            matches[2*found+1]=(i+matched) as u32;
            found+=1; i+=matched;
        }else{
            i+=match source[i]{0..=127=>1,128..=223=>2,224..=239=>3,_=>4};
        }
    }
    found
}
