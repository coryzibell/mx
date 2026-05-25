



.intel_syntax noprefix
.text
.p2align 5
.globl fma_tanh_f32_0_22_1
fma_tanh_f32_0_22_1:
.cfi_startproc


    push        rbp
    mov         rbp, rsp




    push        rbx
    push        r12
    push        r13
    push        r14
    push        r15

    sub         rsp, 8


// FIXME
// .cfi_def_cfa_offset 64 


    stmxcsr     [rsp + 4]

    mov         rax, 0x1FC0

    mov         [rsp], eax
    ldmxcsr     [rsp]
// ----------------------------------------------------------------------



    cmp     rsi, 0
    je      .Ldone

    cmp     rsi, 32
    jl      .Lloop_1

.Lloop_4:

    vmovaps         ymm4, [rdi]
    vmovaps         ymm5, [rdi + 32]
    vmovaps         ymm6, [rdi + 64]
    vmovaps         ymm7, [rdi + 96]

    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_low]
    vbroadcastss    ymm1, dword ptr [ rip +   .Lcoeffs_num_high]
    vbroadcastss    ymm2, dword ptr [ rip +   .Lcoeffs_num_alpha_13]
    vbroadcastss    ymm3, dword ptr [ rip +   .Lcoeffs_num_alpha_11]

    vmaxps          ymm4, ymm4, ymm0
    vmaxps          ymm5, ymm5, ymm0
    vmaxps          ymm6, ymm6, ymm0
    vmaxps          ymm7, ymm7, ymm0
    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_alpha_9]

    vminps          ymm4, ymm4, ymm1
    vminps          ymm5, ymm5, ymm1
    vminps          ymm6, ymm6, ymm1
    vminps          ymm7, ymm7, ymm1        // ymm4..7 <- x
    vbroadcastss    ymm1, dword ptr [ rip +   .Lcoeffs_num_alpha_7]

    vmulps          ymm8, ymm4, ymm4
    vmulps          ymm9, ymm5, ymm5
    vmulps          ymm10, ymm6, ymm6
    vmulps          ymm11, ymm7, ymm7        // ymm8..11 <- x^2

    vmovaps         ymm12, ymm2
    vmovaps         ymm13, ymm2
    vmovaps         ymm14, ymm2
    vmovaps         ymm15, ymm2
    vbroadcastss    ymm2, dword ptr [ rip +   .Lcoeffs_num_alpha_5]
    vfmadd132ps     ymm12, ymm3, ymm8
    vfmadd132ps     ymm13, ymm3, ymm9
    vfmadd132ps     ymm14, ymm3, ymm10
    vfmadd132ps     ymm15, ymm3, ymm11
    vbroadcastss    ymm3, dword ptr [ rip +   .Lcoeffs_num_alpha_3]
    vfmadd132ps     ymm12, ymm0, ymm8
    vfmadd132ps     ymm13, ymm0, ymm9
    vfmadd132ps     ymm14, ymm0, ymm10
    vfmadd132ps     ymm15, ymm0, ymm11
    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_alpha_1]
    vfmadd132ps     ymm12, ymm1, ymm8
    vfmadd132ps     ymm13, ymm1, ymm9
    vfmadd132ps     ymm14, ymm1, ymm10
    vfmadd132ps     ymm15, ymm1, ymm11
    vbroadcastss    ymm1, dword ptr [ rip +   .Lcoeffs_num_beta_6]
    vfmadd132ps     ymm12, ymm2, ymm8
    vfmadd132ps     ymm13, ymm2, ymm9
    vfmadd132ps     ymm14, ymm2, ymm10
    vfmadd132ps     ymm15, ymm2, ymm11
    vbroadcastss    ymm2, dword ptr [ rip +   .Lcoeffs_num_beta_4]
    vfmadd132ps     ymm12, ymm3, ymm8
    vfmadd132ps     ymm13, ymm3, ymm9
    vfmadd132ps     ymm14, ymm3, ymm10
    vfmadd132ps     ymm15, ymm3, ymm11
    vbroadcastss    ymm3, dword ptr [ rip +   .Lcoeffs_num_beta_2]
    vfmadd132ps     ymm12, ymm0, ymm8
    vfmadd132ps     ymm13, ymm0, ymm9
    vfmadd132ps     ymm14, ymm0, ymm10
    vfmadd132ps     ymm15, ymm0, ymm11
    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_beta_0]
    vmulps          ymm4, ymm4, ymm12
    vmulps          ymm5, ymm5, ymm13
    vmulps          ymm6, ymm6, ymm14
    vmulps          ymm7, ymm7, ymm15   // ymm4..7 <- num

    vmovaps         ymm12, ymm1
    vmovaps         ymm13, ymm1
    vmovaps         ymm14, ymm1
    vmovaps         ymm15, ymm1
    vfmadd132ps     ymm12, ymm2, ymm8
    vfmadd132ps     ymm13, ymm2, ymm9
    vfmadd132ps     ymm14, ymm2, ymm10
    vfmadd132ps     ymm15, ymm2, ymm11
    vfmadd132ps     ymm12, ymm3, ymm8
    vfmadd132ps     ymm13, ymm3, ymm9
    vfmadd132ps     ymm14, ymm3, ymm10
    vfmadd132ps     ymm15, ymm3, ymm11
    vfmadd132ps     ymm12, ymm0, ymm8
    vfmadd132ps     ymm13, ymm0, ymm9
    vfmadd132ps     ymm14, ymm0, ymm10
    vfmadd132ps     ymm15, ymm0, ymm11  // ymm12..14 <- denum

    vdivps          ymm4, ymm4, ymm12
    vdivps          ymm5, ymm5, ymm13
    vdivps          ymm6, ymm6, ymm14
    vdivps          ymm7, ymm7, ymm15

    vmovaps [rdi], ymm4
    vmovaps [rdi + 32], ymm5
    vmovaps [rdi + 64], ymm6
    vmovaps [rdi + 96], ymm7

    add     rdi, 128
    sub     rsi, 32
    cmp     rsi, 32
    jg      .Lloop_4

    cmp     rsi, 0
    je      .Ldone

.Lloop_1:
    vmovaps         ymm4, [rdi]

    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_low]
    vbroadcastss    ymm1, dword ptr [ rip +   .Lcoeffs_num_high]
    vbroadcastss    ymm2, dword ptr [ rip +   .Lcoeffs_num_alpha_13]
    vbroadcastss    ymm3, dword ptr [ rip +   .Lcoeffs_num_alpha_11]

    vmaxps          ymm4, ymm4, ymm0
    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_alpha_9]

    vminps          ymm4, ymm4, ymm1        // ymm4 <- x
    vbroadcastss    ymm1, dword ptr [ rip +   .Lcoeffs_num_alpha_7]

    vmulps          ymm8, ymm4, ymm4        // ymm8 <- x^2

    vmovaps         ymm12, ymm2
    vbroadcastss    ymm2, dword ptr [ rip +   .Lcoeffs_num_alpha_5]
    vfmadd132ps     ymm12, ymm3, ymm8
    vbroadcastss    ymm3, dword ptr [ rip +   .Lcoeffs_num_alpha_3]
    vfmadd132ps     ymm12, ymm0, ymm8
    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_alpha_1]
    vfmadd132ps     ymm12, ymm1, ymm8
    vbroadcastss    ymm1, dword ptr [ rip +   .Lcoeffs_num_beta_6]
    vfmadd132ps     ymm12, ymm2, ymm8
    vbroadcastss    ymm2, dword ptr [ rip +   .Lcoeffs_num_beta_4]
    vfmadd132ps     ymm12, ymm3, ymm8
    vbroadcastss    ymm3, dword ptr [ rip +   .Lcoeffs_num_beta_2]
    vfmadd132ps     ymm12, ymm0, ymm8
    vbroadcastss    ymm0, dword ptr [ rip +   .Lcoeffs_num_beta_0]
    vmulps          ymm4, ymm4, ymm12

    vmovaps         ymm12, ymm1
    vfmadd132ps     ymm12, ymm2, ymm8
    vfmadd132ps     ymm12, ymm3, ymm8
    vfmadd132ps     ymm12, ymm0, ymm8

    vdivps          ymm4, ymm4, ymm12

    vmovaps [rdi], ymm4
    add     rdi, 32
    sub     rsi, 8
    jnz     .Lloop_1

.Ldone:

// ----------------------------------------------------------------------

    ldmxcsr     [rsp + 4]

    add         rsp, 8

    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx



    mov rsp, rbp
    pop rbp
    ret



.Lcoeffs_num_low:
     .float  -8.9
.Lcoeffs_num_high:
     .float  8.9

.Lcoeffs_num_alpha_13:
     .float  -8.488492677e-14
.Lcoeffs_num_alpha_11:
     .float  5.277853000e-11
.Lcoeffs_num_alpha_9:
     .float  -2.022500419e-8
.Lcoeffs_num_alpha_7:
     .float  0.00001115424833
.Lcoeffs_num_alpha_5:
     .float  0.003103950131
.Lcoeffs_num_alpha_3:
     .float  0.1308400453
.Lcoeffs_num_alpha_1:
     .float  0.9999999934

.Lcoeffs_num_beta_6:
     .float  0.0002546136580
.Lcoeffs_num_beta_4:
     .float  0.02449515379
.Lcoeffs_num_beta_2:
     .float  0.4641733162
.Lcoeffs_num_beta_0:
     .float  1.0




.cfi_endproc

