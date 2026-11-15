/* SamarOS — 32-bit kernel entry stub.
 *
 * Stage 2 jumps here with a flat GDT already loaded, interrupts off,
 * and the boot information block sitting at physical 0x5000.
 */

    .code32
    .section .entry, "ax"
    .globl _start
_start:
    cld
    movl    $0x0009F000, %esp
    movl    %esp, %ebp

    /* zero .bss */
    movl    $__bss_start, %edi
    movl    $__bss_end, %ecx
    subl    %edi, %ecx
    xorl    %eax, %eax
    rep     stosb

    call    kmain

.hang:
    hlt
    jmp     .hang

/* io helpers used by the C layer ------------------------------------ */
    .text
    .globl k_inb
k_inb:
    movl    4(%esp), %edx
    xorl    %eax, %eax
    inb     %dx, %al
    ret

    .globl k_outb
k_outb:
    movl    4(%esp), %edx
    movl    8(%esp), %eax
    outb    %al, %dx
    ret

    .globl k_inw
k_inw:
    movl    4(%esp), %edx
    xorl    %eax, %eax
    inw     %dx, %ax
    ret

    .globl k_outw
k_outw:
    movl    4(%esp), %edx
    movl    8(%esp), %eax
    outw    %ax, %dx
    ret

    .globl k_rdtsc_lo
k_rdtsc_lo:
    rdtsc
    ret
