/* SamarOS — Stage 2 boot loader (real mode -> 32-bit protected mode)
 *
 * Runs at 0x0000:0x7E00.  Responsibilities:
 *   1. load the kernel image (flat binary) to 0x00020000
 *   2. detect RAM (int 15h, AX=E801h)
 *   3. find + set a 32bpp VESA linear-framebuffer mode (1024x768 preferred)
 *   4. enable A20, install a flat GDT, enter protected mode
 *   5. jump to the kernel entry point at 0x00020000
 *
 * Boot information is handed to the kernel in a struct at 0x00005000.
 *
 * KERNEL_SECTORS / KERNEL_LBA are supplied by the build script (--defsym).
 */

    .set BOOTINFO,   0x5000
    .set VBEINFO,    0x1000          /* 512 byte VbeInfoBlock            */
    .set MODEINFO,   0x1400          /* 256 byte ModeInfoBlock           */
    /* Both live well below stage 2 itself (0x7E00): the BIOS writes 768
       bytes here, and parking them at 0x8000 would shred our own GDT. */
    .set KERNEL_SEG, 0x2000          /* 0x2000:0000 == 0x00020000        */

    .code16
    .section .text
    .globl _start2

_start2:
    cli
    xorw    %ax, %ax
    movw    %ax, %ds
    movw    %ax, %es
    movw    %ax, %fs
    movw    %ax, %gs
    movw    %ax, %ss
    movw    $0x7C00, %sp
    sti

    movb    %dl, boot_drive

    movw    $msg_load, %si
    call    puts

/* ------------------------------------------------------------------ *
 *  1. load kernel                                                     *
 * ------------------------------------------------------------------ */
    movw    $KERNEL_SECTORS, sectors_left
    movw    $KERNEL_SEG, dest_seg
    movw    $KERNEL_LBA, dap_lba

load_loop:
    movw    sectors_left, %ax
    testw   %ax, %ax
    jz      load_done
    cmpw    $32, %ax                 /* 32 sectors = 16 KiB per request  */
    jbe     1f
    movw    $32, %ax
1:  movw    %ax, dap_count
    movw    $0, dap_off
    movw    dest_seg, %bx
    movw    %bx, dap_seg

    movw    $dap, %si
    movb    $0x42, %ah
    movb    boot_drive, %dl
    int     $0x13
    jc      disk_error

    movw    dap_count, %ax
    subw    %ax, sectors_left
    addw    %ax, dap_lba
    shlw    $5, %ax                  /* 512 bytes == 32 paragraphs       */
    addw    %ax, dest_seg
    movb    $'.', %al
    call    putc
    jmp     load_loop

load_done:
    call    enable_a20
    movw    $msg_ok, %si
    call    puts

/* ------------------------------------------------------------------ *
 *  2. memory map (int 15h AX=E801h)                                   *
 * ------------------------------------------------------------------ */
    xorw    %cx, %cx
    xorw    %dx, %dx
    movw    $0xE801, %ax
    int     $0x15
    jc      mem_done
    testw   %cx, %cx
    jz      1f
    movw    %cx, %ax
    movw    %dx, %bx
1:  movw    %ax, mem_low_kb
    movw    %bx, mem_high_64k
mem_done:

/* ------------------------------------------------------------------ *
 *  3. VESA BIOS Extensions: pick the best 32bpp LFB mode              *
 * ------------------------------------------------------------------ */
    xorw    %ax, %ax
    movw    %ax, %es
    movw    $VBEINFO, %di
    movl    $0x32454256, %es:(%di)   /* 'VBE2' request                   */
    movw    $0x4F00, %ax
    int     $0x10
    cmpw    $0x004F, %ax
    jne     vbe_fail

    movw    VBEINFO + 14, %ax        /* VideoModePtr offset              */
    movw    %ax, mode_off
    movw    VBEINFO + 16, %ax        /* VideoModePtr segment             */
    movw    %ax, mode_seg

next_mode:
    movw    mode_seg, %ax
    movw    %ax, %fs
    movw    mode_off, %si
    movw    %fs:(%si), %cx
    addw    $2, %si
    movw    %si, mode_off
    cmpw    $0xFFFF, %cx
    je      modes_done

    xorw    %ax, %ax
    movw    %ax, %es
    movw    $MODEINFO, %di
    movw    $0x4F01, %ax
    movw    %cx, cur_mode
    int     $0x10
    cmpw    $0x004F, %ax
    jne     next_mode

    movw    MODEINFO, %ax            /* ModeAttributes                   */
    testw   $0x0080, %ax             /* linear framebuffer available     */
    jz      next_mode
    testw   $0x0010, %ax             /* graphics mode                    */
    jz      next_mode
    cmpb    $32, MODEINFO + 25       /* BitsPerPixel                     */
    jne     next_mode
    cmpb    $6, MODEINFO + 27        /* MemoryModel == direct colour     */
    jne     next_mode

    movw    MODEINFO + 18, %ax       /* XResolution                      */
    movw    MODEINFO + 20, %bx       /* YResolution                      */

    movw    $0, %dx                  /* rank                             */
    cmpw    $640, %ax
    jne     1f
    cmpw    $480, %bx
    jne     1f
    movw    $1, %dx
1:  cmpw    $800, %ax
    jne     2f
    cmpw    $600, %bx
    jne     2f
    movw    $2, %dx
2:  cmpw    $1024, %ax
    jne     3f
    cmpw    $768, %bx
    jne     3f
    movw    $3, %dx
3:  testw   %dx, %dx
    jz      next_mode
    cmpw    best_rank, %dx
    jbe     next_mode
    movw    %dx, best_rank
    movw    cur_mode, %cx
    movw    %cx, best_mode
    jmp     next_mode

modes_done:
    movw    best_rank, %ax
    testw   %ax, %ax
    jz      vbe_fail

    /* re-query the winning mode so MODEINFO holds its data */
    xorw    %ax, %ax
    movw    %ax, %es
    movw    $MODEINFO, %di
    movw    best_mode, %cx
    movw    $0x4F01, %ax
    int     $0x10

    /* fill the boot information block */
    movw    $BOOTINFO, %di
    movl    $0x524D4153, %es:0(%di)          /* 'SAMR'                    */
    movl    MODEINFO + 40, %eax              /* PhysBasePtr               */
    movl    %eax, %es:4(%di)
    xorl    %eax, %eax
    movw    MODEINFO + 16, %ax               /* BytesPerScanLine          */
    movl    %eax, %es:8(%di)
    movw    MODEINFO + 18, %ax
    movl    %eax, %es:12(%di)                /* width                     */
    movw    MODEINFO + 20, %ax
    movl    %eax, %es:16(%di)                /* height                    */
    movl    $32, %es:20(%di)                 /* bpp                       */
    xorl    %eax, %eax
    movw    mem_low_kb, %ax
    movl    %eax, %es:24(%di)
    xorl    %eax, %eax
    movw    mem_high_64k, %ax
    movl    %eax, %es:28(%di)
    xorl    %eax, %eax
    movb    boot_drive, %al
    movl    %eax, %es:32(%di)

    /* switch the display into the chosen mode (linear frame buffer) */
    movw    best_mode, %bx
    orw     $0x4000, %bx
    movw    $0x4F02, %ax
    int     $0x10
    cmpw    $0x004F, %ax
    jne     vbe_fail

/* ------------------------------------------------------------------ *
 *  4. A20 + protected mode                                            *
 * ------------------------------------------------------------------ */

    cli
    lgdt    gdt_desc
    movl    %cr0, %eax
    orl     $1, %eax
    movl    %eax, %cr0
    ljmp    $0x08, $pm_entry

vbe_fail:
    movw    $msg_vbe, %si
    call    puts
    jmp     halt16

disk_error:
    movw    $msg_disk, %si
    call    puts
halt16:
    cli
1:  hlt
    jmp     1b

/* A20: BIOS service first, 8042 controller as the fallback.  Writing the
 * fast-A20 port 0x92 directly is avoided: some machines (and emulators)
 * treat bit 0 as a system reset. */
enable_a20:
    movw    $0x2401, %ax
    int     $0x15
    jnc     1f
    call    a20_wait_in
    movb    $0xAD, %al
    outb    %al, $0x64
    call    a20_wait_in
    movb    $0xD0, %al
    outb    %al, $0x64
    call    a20_wait_out
    inb     $0x60, %al
    pushw   %ax
    call    a20_wait_in
    movb    $0xD1, %al
    outb    %al, $0x64
    call    a20_wait_in
    popw    %ax
    orb     $0x02, %al
    outb    %al, $0x60
    call    a20_wait_in
    movb    $0xAE, %al
    outb    %al, $0x64
    call    a20_wait_in
1:  ret

a20_wait_in:
    inb     $0x64, %al
    testb   $0x02, %al
    jnz     a20_wait_in
    ret

a20_wait_out:
    inb     $0x64, %al
    testb   $0x01, %al
    jz      a20_wait_out
    ret

/* ---- tiny real mode console helpers ---- */
puts:
    pushw   %ax
    pushw   %bx
1:  lodsb
    testb   %al, %al
    jz      2f
    call    putc
    jmp     1b
2:  popw    %bx
    popw    %ax
    ret

putc:
    pushw   %bx
    movb    $0x0E, %ah
    movw    $0x0007, %bx
    int     $0x10
    popw    %bx
    ret

/* ------------------------------------------------------------------ *
 *  5. 32-bit land                                                     *
 * ------------------------------------------------------------------ */
    .code32
pm_entry:
    movw    $0x10, %ax
    movw    %ax, %ds
    movw    %ax, %es
    movw    %ax, %fs
    movw    %ax, %gs
    movw    %ax, %ss
    movl    $0x0009F000, %esp
    ljmp    $0x08, $0x00020000

/* ---- data ---- */
    .code16
    .align 8
gdt:
    .quad   0x0000000000000000       /* null                             */
    .quad   0x00CF9A000000FFFF       /* 0x08: code, base 0, limit 4G     */
    .quad   0x00CF92000000FFFF       /* 0x10: data, base 0, limit 4G     */
gdt_end:

gdt_desc:
    .word   gdt_end - gdt - 1
    .long   gdt

boot_drive:     .byte 0x80
    .align 2
sectors_left:   .word 0
dest_seg:       .word 0
mode_off:       .word 0
mode_seg:       .word 0
cur_mode:       .word 0
best_mode:      .word 0
best_rank:      .word 0
mem_low_kb:     .word 0
mem_high_64k:   .word 0

    .align 4
dap:
    .byte   0x10
    .byte   0x00
dap_count:  .word 0
dap_off:    .word 0
dap_seg:    .word 0
dap_lba:    .quad 0

msg_load:   .asciz "SamarOS loading"
msg_ok:     .asciz " ok\r\n"
msg_vbe:    .asciz "\r\nNo 32bpp VESA mode\r\n"
msg_disk:   .asciz "\r\nDisk error\r\n"
