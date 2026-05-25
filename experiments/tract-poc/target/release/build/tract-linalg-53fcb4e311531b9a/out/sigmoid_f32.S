



.intel_syntax noprefix
.text
.p2align 5
.globl avx512_sigmoid_f32_0_22_1
avx512_sigmoid_f32_0_22_1:
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

    vmovaps         zmm4, [rdi]
    vmovaps         zmm5, [rdi + 64]
    vmovaps         zmm6, [rdi + 128]
    vmovaps         zmm7, [rdi + 192]

    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_low]
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_high]
    vbroadcastss    zmm2, dword ptr [ rip +   .Lcoeffs_num_alpha_9]
    vbroadcastss    zmm3, dword ptr [ rip +   .Lcoeffs_num_alpha_7]

    vmaxps          zmm4, zmm4, zmm0
    vmaxps          zmm5, zmm5, zmm0
    vmaxps          zmm6, zmm6, zmm0
    vmaxps          zmm7, zmm7, zmm0
    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_alpha_5]

    vminps          zmm4, zmm4, zmm1
    vminps          zmm5, zmm5, zmm1
    vminps          zmm6, zmm6, zmm1
    vminps          zmm7, zmm7, zmm1        // zmm4..7 <- x
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_alpha_3]

    vmulps          zmm8, zmm4, zmm4
    vmulps          zmm9, zmm5, zmm5
    vmulps          zmm10, zmm6, zmm6
    vmulps          zmm11, zmm7, zmm7        // zmm8..11 <- x^2

    vmovaps         zmm12, zmm2
    vmovaps         zmm13, zmm2
    vmovaps         zmm14, zmm2
    vmovaps         zmm15, zmm2
    vbroadcastss    zmm2, dword ptr [ rip +   .Lcoeffs_num_alpha_1]
    vfmadd132ps     zmm12, zmm3, zmm8
    vfmadd132ps     zmm13, zmm3, zmm9
    vfmadd132ps     zmm14, zmm3, zmm10
    vfmadd132ps     zmm15, zmm3, zmm11
    vbroadcastss    zmm3, dword ptr [ rip +   .Lcoeffs_num_beta_10]
    vfmadd132ps     zmm12, zmm0, zmm8
    vfmadd132ps     zmm13, zmm0, zmm9
    vfmadd132ps     zmm14, zmm0, zmm10
    vfmadd132ps     zmm15, zmm0, zmm11
    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_beta_8]
    vfmadd132ps     zmm12, zmm1, zmm8
    vfmadd132ps     zmm13, zmm1, zmm9
    vfmadd132ps     zmm14, zmm1, zmm10
    vfmadd132ps     zmm15, zmm1, zmm11
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_beta_6]
    vfmadd132ps     zmm12, zmm2, zmm8
    vfmadd132ps     zmm13, zmm2, zmm9
    vfmadd132ps     zmm14, zmm2, zmm10
    vfmadd132ps     zmm15, zmm2, zmm11
    vbroadcastss    zmm2, dword ptr [ rip +   .Lcoeffs_num_beta_4]
    vmulps          zmm4, zmm4, zmm12
    vmulps          zmm5, zmm5, zmm13
    vmulps          zmm6, zmm6, zmm14
    vmulps          zmm7, zmm7, zmm15   // zmm4..7 <- num

    vmovaps         zmm12, zmm3
    vmovaps         zmm13, zmm3
    vmovaps         zmm14, zmm3
    vmovaps         zmm15, zmm3
    vbroadcastss    zmm3, dword ptr [ rip +   .Lcoeffs_num_beta_2]
    vfmadd132ps     zmm12, zmm0, zmm8
    vfmadd132ps     zmm13, zmm0, zmm9
    vfmadd132ps     zmm14, zmm0, zmm10
    vfmadd132ps     zmm15, zmm0, zmm11
    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_beta_0]
    vfmadd132ps     zmm12, zmm1, zmm8
    vfmadd132ps     zmm13, zmm1, zmm9
    vfmadd132ps     zmm14, zmm1, zmm10
    vfmadd132ps     zmm15, zmm1, zmm11
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_half]
    vfmadd132ps     zmm12, zmm2, zmm8
    vfmadd132ps     zmm13, zmm2, zmm9
    vfmadd132ps     zmm14, zmm2, zmm10
    vfmadd132ps     zmm15, zmm2, zmm11
    vfmadd132ps     zmm12, zmm3, zmm8
    vfmadd132ps     zmm13, zmm3, zmm9
    vfmadd132ps     zmm14, zmm3, zmm10
    vfmadd132ps     zmm15, zmm3, zmm11
    vfmadd132ps     zmm12, zmm0, zmm8
    vfmadd132ps     zmm13, zmm0, zmm9
    vfmadd132ps     zmm14, zmm0, zmm10
    vfmadd132ps     zmm15, zmm0, zmm11  // zmm12..14 <- denum

    vdivps          zmm4, zmm4, zmm12
    vdivps          zmm5, zmm5, zmm13
    vdivps          zmm6, zmm6, zmm14
    vdivps          zmm7, zmm7, zmm15
    vaddps          zmm4, zmm4, zmm1
    vaddps          zmm5, zmm5, zmm1
    vaddps          zmm6, zmm6, zmm1
    vaddps          zmm7, zmm7, zmm1

    vmovaps [rdi], zmm4
    vmovaps [rdi + 64], zmm5
    vmovaps [rdi + 128], zmm6
    vmovaps [rdi + 192], zmm7

    add     rdi, 256
    sub     rsi, 32
    cmp     rsi, 32
    jg      .Lloop_4

    cmp     rsi, 0
    je      .Ldone

.Lloop_1:
    vmovaps         zmm4, [rdi]

    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_low]
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_high]
    vbroadcastss    zmm2, dword ptr [ rip +   .Lcoeffs_num_alpha_9]
    vbroadcastss    zmm3, dword ptr [ rip +   .Lcoeffs_num_alpha_7]

    vmaxps          zmm4, zmm4, zmm0
    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_alpha_5]

    vminps          zmm4, zmm4, zmm1        // zmm4 <- x
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_alpha_3]

    vmulps          zmm8, zmm4, zmm4        // zmm8 <- x^2

    vmovaps         zmm12, zmm2
    vbroadcastss    zmm2, dword ptr [ rip +   .Lcoeffs_num_alpha_1]
    vfmadd132ps     zmm12, zmm3, zmm8
    vbroadcastss    zmm3, dword ptr [ rip +   .Lcoeffs_num_beta_10]
    vfmadd132ps     zmm12, zmm0, zmm8
    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_beta_8]
    vfmadd132ps     zmm12, zmm1, zmm8
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_beta_6]
    vfmadd132ps     zmm12, zmm2, zmm8
    vbroadcastss    zmm2, dword ptr [ rip +   .Lcoeffs_num_beta_4]
    vmulps          zmm4, zmm4, zmm12

    vmovaps         zmm12, zmm3
    vbroadcastss    zmm3, dword ptr [ rip +   .Lcoeffs_num_beta_2]
    vfmadd132ps     zmm12, zmm0, zmm8
    vbroadcastss    zmm0, dword ptr [ rip +   .Lcoeffs_num_beta_0]
    vfmadd132ps     zmm12, zmm1, zmm8
    vbroadcastss    zmm1, dword ptr [ rip +   .Lcoeffs_num_half]
    vfmadd132ps     zmm12, zmm2, zmm8
    vfmadd132ps     zmm12, zmm3, zmm8
    vfmadd132ps     zmm12, zmm0, zmm8

    vdivps          zmm4, zmm4, zmm12
    vaddps          zmm4, zmm4, zmm1

    vmovaps [rdi], zmm4
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
     .float  -18.0                    // low
.Lcoeffs_num_high:
     .float  18.0                     // high

.Lcoeffs_num_alpha_9:
     .float  4.37031012579801e-11     // alpha_9
.Lcoeffs_num_alpha_7:
     .float  1.15627324459942e-07     // alpha_7
.Lcoeffs_num_alpha_5:
     .float  6.08574864600143e-05     // alpha_5
.Lcoeffs_num_alpha_3:
     .float  8.51377133304701e-03     // alpha_3
.Lcoeffs_num_alpha_1:
     .float  2.48287947061529e-01     // alpha_1

.Lcoeffs_num_beta_10:
     .float  6.10247389755681e-13
.Lcoeffs_num_beta_8:
     .float  5.76102136993427e-09
.Lcoeffs_num_beta_6:
     .float  6.29106785017040e-06     // beta_6
.Lcoeffs_num_beta_4:
     .float  1.70198817374094e-03     // beta_4
.Lcoeffs_num_beta_2:
     .float  1.16817656904453e-01     // beta_2
.Lcoeffs_num_beta_0:
     .float  9.93151921023180e-01     // beta_0

.Lcoeffs_num_half:
     .float  0.5


.cfi_endproc

