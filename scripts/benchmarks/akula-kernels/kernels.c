/* Standalone ports of four kernels used in Akula.
 * Provenance:
 * native/src/render/paint/mipmap.rs::half
 * native/src/graph/raster/mod.rs::dilate
 * core/src/sync/text_delta_codec.rs::splice and splice_apply
 * core/src/text/matcher/mod.rs::Matcher::each (Fold::Exact, substring only)
 *
 * Preconditions: valid nonaliasing buffers, valid UTF-8 text/needles,
 * sizes <= 2 MiB for splice, <= UINT32_MAX offsets for search.
 * Search patterns are sorted by descending Unicode scalar length, then lexical order.
 */
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#ifdef __cplusplus
extern "C" {
#endif

static uint8_t maximum(uint8_t a, uint8_t b) { return a > b ? a : b; }
static size_t minimum(size_t a, size_t b) { return a < b ? a : b; }

uint32_t ak_mipmap(const uint8_t *src, uint8_t *dst, size_t w, size_t h) {
    if (!w || !h || (w == 1 && h == 1)) return 0;
    const size_t dw = (w+1)/2, dh = (h+1)/2;
    uint32_t transparency = 0;
    for (size_t y=0; y<dh; y++) {
        const size_t y0=2*y, y1=minimum(y0+1,h-1);
        for (size_t x=0; x<dw; x++) {
            const size_t x0=2*x, x1=minimum(x0+1,w-1);
            const size_t a=4*(y0*w+x0), b=4*(y0*w+x1);
            const size_t c=4*(y1*w+x0), d=4*(y1*w+x1);
            for (size_t ch=0; ch<4; ch++) {
                const uint32_t sum=(uint32_t)src[a+ch]+src[b+ch]+src[c+ch]+src[d+ch];
                const uint8_t v=(uint8_t)((sum+2)/4);
                dst[4*(y*dw+x)+ch]=v;
                if (ch==3 && v!=255) transparency=1;
            }
        }
    }
    return transparency ? 2 : 1;
}

void ak_dilate(const uint8_t *mask, uint8_t *horizontal, uint8_t *out, size_t w, size_t h) {
    if (!w || !h) return;
    for (size_t y=0; y<h; y++) {
        const size_t row=y*w;
        for (size_t x=0; x<w; x++)
            horizontal[row+x] = maximum(mask[row+x],
                maximum(mask[row+(x?x-1:x)],mask[row+minimum(x+1,w-1)]));
    }
    for (size_t y=0; y<h; y++) {
        const size_t prev=(y?y-1:y)*w, row=y*w, next=minimum(y+1,h-1)*w;
        for (size_t x=0; x<w; x++)
            out[row+x]=maximum(horizontal[prev+x],
                maximum(horizontal[row+x],horizontal[next+x]));
    }
}

static void store32(uint8_t *dst, uint32_t v) {
    dst[0]=(uint8_t)v; dst[1]=(uint8_t)(v>>8);
    dst[2]=(uint8_t)(v>>16); dst[3]=(uint8_t)(v>>24);
}
static uint32_t load32(const uint8_t *src) {
    return (uint32_t)src[0] | ((uint32_t)src[1]<<8)
        | ((uint32_t)src[2]<<16) | ((uint32_t)src[3]<<24);
}
size_t ak_splice(const uint8_t *base, size_t bn,
                 const uint8_t *target, size_t tn, uint8_t *patch) {
    size_t prefix=0, common=minimum(bn,tn);
    while (prefix<common && base[prefix]==target[prefix]) prefix++;
    size_t suffix=0;
    while (suffix<common-prefix && base[bn-1-suffix]==target[tn-1-suffix]) suffix++;
    const size_t removed=bn-prefix-suffix, inserted=tn-prefix-suffix;
    store32(patch,(uint32_t)prefix);
    store32(patch+4,(uint32_t)removed);
    store32(patch+8,(uint32_t)inserted);
    memcpy(patch+12,target+prefix,inserted);
    return 12+inserted;
}
size_t ak_apply(const uint8_t *base, size_t bn,
                const uint8_t *patch, size_t plen, uint8_t *dst) {
    if (plen<12) return SIZE_MAX;
    const size_t prefix=load32(patch), removed=load32(patch+4), inserted=load32(patch+8);
    if (prefix>bn || removed>bn-prefix || inserted!=plen-12) return SIZE_MAX;
    memcpy(dst,base,prefix);
    memcpy(dst+prefix,patch+12,inserted);
    memcpy(dst+prefix+inserted,base+prefix+removed,bn-prefix-removed);
    return bn-removed+inserted;
}
size_t ak_search(const uint8_t *text, size_t n, const uint8_t *needles,
                 const uint32_t *offsets, size_t count, uint32_t *hits, size_t capacity) {
    size_t found=0;
    for (size_t i=0; i<n;) {
        size_t matched=0;
        for (size_t k=0; k<count; k++) {
            const size_t start=offsets[k], end=offsets[k+1], len=end-start;
            if (len && len<=n-i && text[i]==needles[start] &&
                memcmp(text+i,needles+start,len)==0) {matched=len;break;}
        }
        if (matched) {
            if (found>=capacity) return SIZE_MAX;
            hits[2*found]=(uint32_t)i;
            hits[2*found+1]=(uint32_t)(i+matched);
            found++; i+=matched;
        } else {
            const uint8_t b=text[i];
            i+= b<128?1:(b<224?2:(b<240?3:4)); /* valid UTF-8 only */
        }
    }
    return found;
}
#ifdef __cplusplus
}
#endif
