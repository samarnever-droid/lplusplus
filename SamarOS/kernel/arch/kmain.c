/* SamarOS — kernel bring-up.
 *
 * Stage 2 has already put us in 32-bit protected mode with a flat GDT and a
 * linear framebuffer.  We wire up the heap, the display and the PS/2
 * controller, then hand the machine to the L++ kernel (`def main()` in
 * kernel/src/samaros.lpp, emitted as samar_main).
 */
#include "samar.h"
#include "../runtime/kernel_api.h"

bootinfo_t *g_boot;

static u32 mem_kb;
static int cpu_mhz_val;

int mem_total_kb(void) { return (int)mem_kb; }
int boot_drive(void)   { return (int)g_boot->boot_drive; }
int cpu_mhz(void)      { return cpu_mhz_val; }

void cpu_halt(void) { for (;;) __asm__ __volatile__("hlt"); }

/* bochs/qemu style debug console — invaluable when there is no screen yet */
void debug_out(const char *s)
{
    while (s && *s) k_outb(0xE9, (u32)(u8)*s++);
}

/* rough TSC based clock estimate, measured against the PIT */
static void measure_cpu(void)
{
    int t0 = time_ms();
    while (time_ms() == t0) input_poll();
    u32 c0 = k_rdtsc_lo();
    int start = time_ms();
    while (time_ms() - start < 30) input_poll();
    u32 c1 = k_rdtsc_lo();
    u32 elapsed = c1 - c0;
    cpu_mhz_val = (int)(elapsed / 30000u);
}

void kmain(void)
{
    g_boot = (bootinfo_t *)BOOTINFO_ADDR;

    if (g_boot->magic != 0x524D4153u) {   /* 'SAMR' */
        debug_out("SamarOS: bad boot information block\n");
        cpu_halt();
    }

    gfx_init();

    u32 heap_end = HEAP_MIN_END;
    mem_kb = 1024;
    if (g_boot->mem_low_kb)   mem_kb += g_boot->mem_low_kb;
    if (g_boot->mem_high_64k) mem_kb += g_boot->mem_high_64k * 64u;
    if (mem_kb > 1024u) {
        u32 top = mem_kb * 1024u;
        if (top > 0x20000000u) top = 0x20000000u;   /* cap the heap at 512 MiB */
        if (top > HEAP_START + 0x200000u) heap_end = top;
    }
    heap_init(HEAP_START, heap_end);

    input_init();
    measure_cpu();

    debug_out("SamarOS: entering L++ kernel\n");
    samar_main();

    cpu_halt();
}
