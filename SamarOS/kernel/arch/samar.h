/* SamarOS — shared kernel definitions (freestanding, no libc) */
#ifndef SAMAR_H
#define SAMAR_H

typedef unsigned char       u8;
typedef signed char         s8;
typedef unsigned short      u16;
typedef short               s16;
typedef unsigned int        u32;
typedef int                 s32;
typedef unsigned long long  u64;

#define NULL ((void *)0)

/* physical memory layout ------------------------------------------- */
#define BOOTINFO_ADDR   0x00005000u
#define BACKBUFFER_ADDR 0x00400000u   /* 4 MiB  — kernel render target   */
#define WALLCACHE_ADDR  0x00800000u   /* 8 MiB  — cached desktop layer   */
#define HEAP_START      0x00C00000u   /* 12 MiB — bump heap              */
#define HEAP_MIN_END    0x01C00000u   /* 28 MiB — assumed if E801 fails  */

typedef struct {
    u32 magic;          /* 'SAMR' */
    u32 fb_addr;
    u32 fb_pitch;
    u32 fb_width;
    u32 fb_height;
    u32 fb_bpp;
    u32 mem_low_kb;     /* KiB between 1 MiB and 16 MiB   */
    u32 mem_high_64k;   /* 64 KiB blocks above 16 MiB     */
    u32 boot_drive;
} bootinfo_t;

extern bootinfo_t *g_boot;

/* port io (entry.S) -------------------------------------------------- */
u32  k_inb(u32 port);
void k_outb(u32 port, u32 value);
u32  k_inw(u32 port);
void k_outw(u32 port, u32 value);
u32  k_rdtsc_lo(void);

/* memory ------------------------------------------------------------- */
void  heap_init(u32 start, u32 end);
void *lpp_alloc(int size);
int   heap_used(void);
int   heap_total(void);
void *k_memset(void *dst, int c, unsigned n);
void *k_memcpy(void *dst, const void *src, unsigned n);

/* graphics ----------------------------------------------------------- */
void gfx_init(void);
int  fb_width(void);
int  fb_height(void);

/* input / time -------------------------------------------------------- */
void input_init(void);
void input_poll(void);

#endif /* SAMAR_H */
