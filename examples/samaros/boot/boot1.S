/* SamarOS — Stage 1 boot sector (MBR, 512 bytes)
 *
 * BIOS loads this at 0x0000:0x7C00 in 16-bit real mode.
 * Job: load Stage 2 from LBA 1 into 0x0000:0x7E00 and jump to it.
 *
 * Assembled with GNU as (AT&T syntax, .code16).
 * STAGE2_SECTORS is supplied by the build script via --defsym.
 */

    .code16
    .section .text
    .globl _start

_start:
    cli
    xorw    %ax, %ax
    movw    %ax, %ds
    movw    %ax, %es
    movw    %ax, %ss
    movw    $0x7C00, %sp
    sti

    movb    %dl, boot_drive          /* BIOS hands us the boot drive in DL */

    /* --- read Stage 2 (LBA 1 .. STAGE2_SECTORS) to 0x0000:0x7E00 --- */
    movw    $STAGE2_SECTORS, dap_count
    movw    $dap, %si
    movb    $0x42, %ah
    movb    boot_drive, %dl
    int     $0x13
    jc      disk_error

    movb    boot_drive, %dl
    ljmp    $0x0000, $0x7E00

disk_error:
    movw    $msg_err, %si
1:  lodsb
    testb   %al, %al
    jz      2f
    movb    $0x0E, %ah
    movw    $0x0007, %bx
    int     $0x10
    jmp     1b
2:  cli
3:  hlt
    jmp     3b

msg_err:
    .asciz  "SamarOS: stage2 read error\r\n"

boot_drive:
    .byte   0x80

    .align 4
dap:
    .byte   0x10                     /* DAP size                        */
    .byte   0x00                     /* reserved                        */
dap_count:
    .word   1                        /* sectors to read                 */
    .word   0x7E00                   /* dest offset                     */
    .word   0x0000                   /* dest segment                    */
    .quad   1                        /* starting LBA                    */

    .org    446                      /* keep partition table area clean  */
    .fill   64, 1, 0
    .word   0xAA55
