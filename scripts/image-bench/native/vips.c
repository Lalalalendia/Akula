#include <vips/vips.h>
#include <time.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static double ms(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec*1000.0+t.tv_nsec/1e6;}
static unsigned clamp(double d,unsigned max){if(d<1)return 1;if(d>=max)return max;return (unsigned)round(d);}
int main(int argc,char **argv){
 if(argc!=3)return 2;
 if(VIPS_INIT(argv[0]))return 3;
 vips_concurrency_set(1);
 VipsImage *input=NULL,*thumb=NULL,*rgba=NULL;
 void *bytes=NULL;size_t len=0;
 double start=ms();
 input=vips_image_new_from_file(argv[1],"access",VIPS_ACCESS_SEQUENTIAL,NULL);
 if(!input||input->Xsize<=0||input->Ysize<=0)goto fail;
 unsigned w=(unsigned)input->Xsize,h=(unsigned)input->Ysize;
 double k=fmin(1600.0/w,sqrt(2560000.0/((double)w*h)));
 unsigned tw=k>=1?w:clamp(w*k,w),th=k>=1?h:clamp(h*k,h);
 if(vips_thumbnail(argv[1],&thumb,(int)tw,"height",(int)th,"size",VIPS_SIZE_FORCE,"no_rotate",TRUE,NULL))goto fail;
 if(thumb->BandFmt!=VIPS_FORMAT_UCHAR)goto fail;
 if(thumb->Bands==4){
   /* Preserve alpha of real PNG/WebP images. */
   rgba=thumb;g_object_ref(rgba);
 } else if(thumb->Bands==3) {
   if(vips_bandjoin_const1(thumb,&rgba,255.0,NULL))goto fail;
 } else {
   fprintf(stderr,"unsupported libvips output bands: %d\n",thumb->Bands);
   goto fail;
 }
 bytes=vips_image_write_to_memory(rgba,&len);
 if(!bytes||len!=(size_t)tw*th*4)goto fail;
 double elapsed=ms()-start;
 FILE *out=fopen(argv[2],"wb");if(!out)goto fail;
 size_t wr=fwrite(bytes,1,len,out);int closed=fclose(out);if(wr!=len||closed)goto fail;
 printf("RESULT mode=vips original=%ux%u shown=%ux%u rgba=%zu read_decode_ms=%.3f\n",w,h,tw,th,len,elapsed);
 g_free(bytes);g_object_unref(rgba);g_object_unref(thumb);g_object_unref(input);vips_shutdown();return 0;
fail:
 if(vips_error_buffer()[0])fprintf(stderr,"%s\n",vips_error_buffer());
 if(bytes)g_free(bytes);if(rgba)g_object_unref(rgba);if(thumb)g_object_unref(thumb);if(input)g_object_unref(input);vips_shutdown();return 3;
}
