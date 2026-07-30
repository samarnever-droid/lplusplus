#ifndef C2LPP_ZLIB_API_H
#define C2LPP_ZLIB_API_H

#include <stddef.h>

#define Z_OK 0
#define Z_STREAM_END 1
#define Z_NEED_DICT 2
#define Z_ERRNO -1
#define Z_DEFAULT_COMPRESSION -1

typedef unsigned char Bytef;
typedef unsigned long uLong;
typedef unsigned long uLongf;

const char *zlibVersion(void);
int compress(Bytef *dest, uLongf *destLen, const Bytef *source, uLong sourceLen);
int compress2(Bytef *dest, uLongf *destLen, const Bytef *source, uLong sourceLen, int level);
int uncompress(Bytef *dest, uLongf *destLen, const Bytef *source, uLong sourceLen);
uLong compressBound(uLong sourceLen);

#endif
