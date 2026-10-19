#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <time.h>
#include <math.h>
#include <setjmp.h>
#include <jpeglib.h>

struct jpeg_error { struct jpeg_error_mgr pub; jmp_buf jump; };
static void fail(j_common_ptr c) { struct jpeg_error *e=(struct jpeg_error *)c->err; (*c->err->output_message)(c); longjmp(e->jump,1); }
static double ms(void) { struct timespec ts; clock_gettime(CLOCK_MONOTONIC,&ts); return (double)ts.tv_sec*1000.0+ts.tv_nsec/1.0e6; }
static unsigned clamp(double f,unsigned max) { if(f<1)return 1; if(f>=max)return max; return (unsigned)round(f); }
static void fit(unsigned w,unsigned h,unsigned *tw,unsigned *th) { double k=fmin(1600.0/w,sqrt(2560000.0/((double)w*h))); if(k>=1){*tw=w;*th=h;}else{*tw=clamp(w*k,w);*th=clamp(h*k,h);} }
int main(int argc,char **argv) {
 if(argc!=3) {fputs("usage: turbo <jpeg> <rgba>\n",stderr); return 2;}
 FILE *in=fopen(argv[1],"rb"); if(!in){perror("input");return 2;}
 struct jpeg_decompress_struct d={0}; struct jpeg_error err={0};
 d.err=jpeg_std_error(&err.pub); err.pub.error_exit=fail;
 jpeg_create_decompress(&d);
 if(setjmp(err.jump)){jpeg_destroy_decompress(&d);fclose(in);return 3;}
 double start=ms(); jpeg_stdio_src(&d,in);
 if(jpeg_read_header(&d,TRUE)!=JPEG_HEADER_OK){jpeg_destroy_decompress(&d);fclose(in);return 3;}
 unsigned w=d.image_width,h=d.image_height,tw,th;fit(w,h,&tw,&th);
 if(!tw||!th||(uint64_t)tw*th>2560000ULL){jpeg_destroy_decompress(&d);fclose(in);return 3;}
 for(int denom=8;denom>=1;denom/=2){d.scale_num=1;d.scale_denom=denom;jpeg_calc_output_dimensions(&d);if(d.output_width>=tw&&d.output_height>=th)break;}
 d.out_color_space=JCS_RGB;
 if(!jpeg_start_decompress(&d)||d.output_components!=3){jpeg_destroy_decompress(&d);fclose(in);return 3;}
 size_t sw=d.output_width,sh=d.output_height;
 if(!sw||!sh||sw>SIZE_MAX/sizeof(uint32_t)|| (size_t)tw>SIZE_MAX/((size_t)th*4))return 3;
 unsigned char *out=malloc((size_t)tw*th*4);
 uint32_t *cols=malloc(sw*sizeof(uint32_t));
 uint64_t *sums=calloc((size_t)tw*5,sizeof(uint64_t));
 if(!out||!cols||!sums)return 3;
 for(size_t x=0;x<sw;x++)cols[x]=(uint32_t)((uint64_t)x*tw/sw);
 JSAMPARRAY rows=(*d.mem->alloc_sarray)((j_common_ptr)&d,JPOOL_IMAGE,(JDIMENSION)(sw*3),1);
 size_t finished=0;
 while(d.output_scanline<d.output_height){
   size_t y=d.output_scanline;
   if(jpeg_read_scanlines(&d,rows,1)!=1)return 3;
   for(size_t x=0;x<sw;x++){
     uint8_t *src=rows[0]+3*x;
     uint64_t *b=&sums[(size_t)cols[x]*5];
     b[0]+=(uint64_t)src[0]*255;b[1]+=(uint64_t)src[1]*255;b[2]+=(uint64_t)src[2]*255;b[3]+=255;b[4]++;
   }
   if(((uint64_t)(y+1)*th/sh)!=((uint64_t)y*th/sh)){
     if(finished>=th)return 3;
     for(size_t x=0;x<tw;x++){
       uint64_t *b=&sums[5*x];size_t k=4*(finished*tw+x);uint64_t a=b[3],cnt=b[4];
       out[k]=a?(uint8_t)((b[0]+a/2)/a):0;
       out[k+1]=a?(uint8_t)((b[1]+a/2)/a):0;
       out[k+2]=a?(uint8_t)((b[2]+a/2)/a):0;
       out[k+3]=cnt?(uint8_t)((a+cnt/2)/cnt):0;
       memset(b,0,5*sizeof(uint64_t));
     }
     finished++;
   }
 }
 double elapsed=ms()-start;
 int progressive=d.progressive_mode;
 jpeg_finish_decompress(&d);jpeg_destroy_decompress(&d);fclose(in);
 if(finished!=th)return 3;
 FILE *f=fopen(argv[2],"wb");if(!f)return 3;
 size_t len=(size_t)tw*th*4;
 if(fwrite(out,1,len,f)!=len||fclose(f))return 3;
 printf("RESULT mode=turbo original=%ux%u shown=%ux%u rgba=%zu read_decode_ms=%.3f progressive=%d\n",w,h,tw,th,len,elapsed,progressive);
 free(out);free(cols);free(sums);
 return 0;
}
