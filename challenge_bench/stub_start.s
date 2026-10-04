    .text
    .globl _start
    .type _start, @function
_start:
    call main
    mov %eax, %edi
    mov $60, %eax
    syscall
