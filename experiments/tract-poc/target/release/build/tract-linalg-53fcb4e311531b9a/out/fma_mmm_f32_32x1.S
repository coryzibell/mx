




.intel_syntax noprefix
.text
.p2align 5
.globl fma_mmm_f32_32x1_0_22_1
fma_mmm_f32_32x1_0_22_1:
.cfi_startproc



    push        rbp
    mov         rbp, rsp



    push        rbx
    push        r12
    push        r13
    push        r14
    push        r15

    sub         rsp, 8


.cfi_def_cfa_offset 64

    stmxcsr     [rsp + 4]

    mov         rax, 0x1FC0

    mov         [rsp], eax
    ldmxcsr     [rsp]

// vim: set syntax=asm :

.Lnon_linear:

.Lnon_linear_loop_enter:
    sub     rdi,    40
.Lnon_linear_loop:
    add     rdi,    40
    mov     rax,    [rdi]

    mov     r8, 29
    cmp     rax, 0
    cmovl   rax, r8
    cmp     rax, 29
    cmovg   rax, r8


    lea     r8, [ rip + .Ljmp_table ]

    movsxd  r9, dword ptr [ r8 + rax * 4 ]
    lea     r8, [ r8 + r9 ]
    jmp     r8

.Ljmp_table:

    .long      .Ldone-.Ljmp_table

    .long      .Lclear-.Ljmp_table

    .long      .Lload_tile-.Ljmp_table

    .long      .Lscalar_min-.Ljmp_table

    .long      .Lscalar_max-.Ljmp_table

    .long      .Lscalar_add-.Ljmp_table

    .long      .Lscalar_mul-.Ljmp_table

    .long      .Lscalar_sub-.Ljmp_table

    .long      .Lscalar_sub_flipped-.Ljmp_table

    .long      .Lleaky_relu-.Ljmp_table

    .long      .Lper_row_min-.Ljmp_table

    .long      .Lper_row_max-.Ljmp_table

    .long      .Lper_row_add-.Ljmp_table

    .long      .Lper_row_mul-.Ljmp_table

    .long      .Lper_row_sub-.Ljmp_table

    .long      .Lper_row_sub_flipped-.Ljmp_table

    .long      .Lper_col_min-.Ljmp_table

    .long      .Lper_col_max-.Ljmp_table

    .long      .Lper_col_add-.Ljmp_table

    .long      .Lper_col_mul-.Ljmp_table

    .long      .Lper_col_sub-.Ljmp_table

    .long      .Lper_col_sub_flipped-.Ljmp_table

    .long      .Lq_scale-.Ljmp_table

    .long      .Lq_shr-.Ljmp_table

    .long      .Lq_shl-.Ljmp_table

    .long      .Ladd_unicast-.Ljmp_table

    .long      .Ladd_row_col_products-.Ljmp_table

    .long      .Lstore-.Ljmp_table

    .long      .Ladd_mat_mul-.Ljmp_table

    .long      .Lunsupported-.Ljmp_table

.Lunsupported:
    mov     rax,    1
    jmp     .Lreturn


.Ldone:
    mov     rax, 0
    jmp     .Lreturn




.Lclear:
    vzeroall
    jmp     .Lnon_linear_loop

.Ladd_mat_mul:
    mov     rcx,    [rdi + 24]   // B
    mov     rax,    [rdi + 16]   // A

    mov     rbx,    [rdi + 8]    // k
    mov     r8,    [rdi + 32]   // packing
    test    rbx,    rbx
    jz      .Lnon_linear_loop

    cmp     r8, 1
    jz      .Lq40f32

    cmp     r8, 2
    jz      .Lq40f16

    cmp     r8, 3
    jz      .Lf16f16

.align 16
.Lmain_loop_packed_packed:
    vbroadcastss    ymm15,  dword ptr [rcx]

    vmovaps     ymm8, [rax]
    vmovaps     ymm9, [rax + 32]
    vmovaps     ymm10, [rax + 64]
    vmovaps     ymm11, [rax + 96]

    vfmadd231ps     ymm0, ymm15, ymm8
    vfmadd231ps     ymm1, ymm15, ymm9
    vfmadd231ps     ymm2, ymm15, ymm10
    vfmadd231ps     ymm3, ymm15, ymm11

    add             rcx, 4
	add             rax, 128
    sub             rbx, 1
    jnz             .Lmain_loop_packed_packed

    jmp             .Lnon_linear_loop


.Lq40f32_mask:
    .long 0x0F0F0F0F
.Lq40f32_eight:
    .long 8


.Lq40f32:
    // ymm0-3: acc
    // ymm4-7: scales
    // ymm13: 8
    // ymm14: mask
    // ymm15: b value
    vbroadcastss    ymm14, dword ptr [rip +  .Lq40f32_mask]
    vbroadcastss    ymm13, dword ptr [rip +  .Lq40f32_eight]

.Lq40f32_outerloop:
    // scales
    vmovaps         xmm4, [rax]
    vmovaps         xmm5, [rax + 16]
    vmovaps         xmm6, [rax + 32]
    vmovaps         xmm7, [rax + 48]
    vcvtph2ps       ymm4, xmm4
    vcvtph2ps       ymm5, xmm5
    vcvtph2ps       ymm6, xmm6
    vcvtph2ps       ymm7, xmm7
    add             rax, 64

    mov             rdx, 32

.Lq40f32_innerloop:
    vbroadcastss    ymm15, dword ptr [rcx]
    vmovaps         xmm8, [rax]            // 32 nibbles

    vpand           xmm10, xmm8, xmm14      // 16 bytes

    vpmovzxbd       ymm9, xmm10            // 8 u32

    vpermilpd       xmm10, xmm10, 1        // swap 64bit halves
    vpmovzxbd       ymm10, xmm10            // 8 u32

    vpsrlw          xmm8, xmm8, 4
    vpand           xmm12, xmm8, xmm14      // 16 bytes
    vpmovzxbd       ymm11, xmm12            // 8 u32
    vpermilpd       xmm12, xmm12, 1        // swap 64bit halves
    vpmovzxbd       ymm12, xmm12            // 8 u32

    vpsubd          ymm9, ymm9, ymm13
    vpsubd          ymm10, ymm10, ymm13
    vpsubd          ymm11, ymm11, ymm13
    vpsubd          ymm12, ymm12, ymm13

    vcvtdq2ps       ymm9, ymm9
    vcvtdq2ps       ymm10, ymm10
    vcvtdq2ps       ymm11, ymm11
    vcvtdq2ps       ymm12, ymm12

    vmulps          ymm9, ymm9, ymm4
    vmulps          ymm10, ymm10, ymm5
    vmulps          ymm11, ymm11, ymm6
    vmulps          ymm12, ymm12, ymm7

    vfmadd231ps     ymm0, ymm15, ymm9
    vfmadd231ps     ymm1, ymm15, ymm10
    vfmadd231ps     ymm2, ymm15, ymm11
    vfmadd231ps     ymm3, ymm15, ymm12

    add             rax, 16
    add             rcx, 4
    sub             rdx, 1
    jnz             .Lq40f32_innerloop

    sub             rbx, 32
    jnz             .Lq40f32_outerloop

    jmp             .Lnon_linear_loop

.Lq40f16:
    // ymm0-3: acc
    // ymm4-7: scales
    // ymm13: 8
    // ymm14: mask
    // ymm15: b value
    vbroadcastss    ymm14, dword ptr [rip +  .Lq40f32_mask]
    vbroadcastss    ymm13, dword ptr [rip +  .Lq40f32_eight]

.Lq40f16_outerloop:
    // scales
    vmovaps         xmm4, [rax]
    vmovaps         xmm5, [rax + 16]
    vmovaps         xmm6, [rax + 32]
    vmovaps         xmm7, [rax + 48]
    vcvtph2ps       ymm4, xmm4
    vcvtph2ps       ymm5, xmm5
    vcvtph2ps       ymm6, xmm6
    vcvtph2ps       ymm7, xmm7
    add             rax, 64

    mov             rdx, 32

.Lq40f16_innerloop:
    vpbroadcastw    ymm15, word ptr [rcx]
    vcvtph2ps       ymm15, xmm15

    vmovaps         xmm8, [rax]            // 32 nibbles

    vpand           xmm10, xmm8, xmm14      // 16 bytes

    vpmovzxbd       ymm9, xmm10            // 8 u32

    vpermilpd       xmm10, xmm10, 1        // swap 64bit halves
    vpmovzxbd       ymm10, xmm10            // 8 u32

    vpsrlw          xmm8, xmm8, 4
    vpand           xmm12, xmm8, xmm14      // 16 bytes
    vpmovzxbd       ymm11, xmm12            // 8 u32
    vpermilpd       xmm12, xmm12, 1        // swap 64bit halves
    vpmovzxbd       ymm12, xmm12            // 8 u32

    vpsubd          ymm9, ymm9, ymm13
    vpsubd          ymm10, ymm10, ymm13
    vpsubd          ymm11, ymm11, ymm13
    vpsubd          ymm12, ymm12, ymm13

    vcvtdq2ps       ymm9, ymm9
    vcvtdq2ps       ymm10, ymm10
    vcvtdq2ps       ymm11, ymm11
    vcvtdq2ps       ymm12, ymm12

    vmulps          ymm9, ymm9, ymm4
    vmulps          ymm10, ymm10, ymm5
    vmulps          ymm11, ymm11, ymm6
    vmulps          ymm12, ymm12, ymm7

    vfmadd231ps     ymm0, ymm15, ymm9
    vfmadd231ps     ymm1, ymm15, ymm10
    vfmadd231ps     ymm2, ymm15, ymm11
    vfmadd231ps     ymm3, ymm15, ymm12

    add             rax, 16
    add             rcx, 2
    sub             rdx, 1
    jnz             .Lq40f16_innerloop

    sub             rbx, 32
    jnz             .Lq40f16_outerloop

    jmp             .Lnon_linear_loop

.Lf16f16:
.align 16
    vpbroadcastw    ymm15, word ptr [rcx]

    vmovaps     xmm4, [rax]
    vmovaps     xmm5, [rax + 16]
    vmovaps     xmm6, [rax + 32]
    vmovaps     xmm7, [rax + 48]

    vcvtph2ps       ymm15, xmm15
    vcvtph2ps       ymm4, xmm4
    vcvtph2ps       ymm5, xmm5
    vcvtph2ps       ymm6, xmm6
    vcvtph2ps       ymm7, xmm7

    vfmadd231ps     ymm0, ymm15, ymm4
    vfmadd231ps     ymm1, ymm15, ymm5
    vfmadd231ps     ymm2, ymm15, ymm6
    vfmadd231ps     ymm3, ymm15, ymm7

    add             rcx, 2
	add             rax, 64
    sub             rbx, 1
    jnz             .Lf16f16

    jmp             .Lnon_linear_loop

// vim: set syntax=asm :

// vim: set syntax=asm :

.Lscalar_min:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vminps          ymm0, ymm12, ymm0
        
            vminps          ymm1, ymm12, ymm1
        
            vminps          ymm2, ymm12, ymm2
        
            vminps          ymm3, ymm12, ymm3
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_max:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vmaxps          ymm0, ymm12, ymm0
        
            vmaxps          ymm1, ymm12, ymm1
        
            vmaxps          ymm2, ymm12, ymm2
        
            vmaxps          ymm3, ymm12, ymm3
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_add:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vaddps          ymm0, ymm12, ymm0
        
            vaddps          ymm1, ymm12, ymm1
        
            vaddps          ymm2, ymm12, ymm2
        
            vaddps          ymm3, ymm12, ymm3
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_mul:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vmulps          ymm0, ymm12, ymm0
        
            vmulps          ymm1, ymm12, ymm1
        
            vmulps          ymm2, ymm12, ymm2
        
            vmulps          ymm3, ymm12, ymm3
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vsubps          ymm0, ymm12, ymm0
        
            vsubps          ymm1, ymm12, ymm1
        
            vsubps          ymm2, ymm12, ymm2
        
            vsubps          ymm3, ymm12, ymm3
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub_flipped:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vsubps          ymm0, ymm0, ymm12
        
            vsubps          ymm1, ymm1, ymm12
        
            vsubps          ymm2, ymm2, ymm12
        
            vsubps          ymm3, ymm3, ymm12
        
    

    jmp    .Lnon_linear_loop


.Lleaky_relu:
    // can only use ymm12 to ymm15
    // ymm15 <- alpha
    
        vbroadcastss    ymm15, dword ptr [rdi + 8]
    

    // ymm14 <- all zero
    vpxor           ymm14, ymm14, ymm14

    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm0, ymm15
        vcmpps     ymm13, ymm14, ymm0, 1 // 1 means LT
        vblendvps   ymm0, ymm12, ymm0, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm1, ymm15
        vcmpps     ymm13, ymm14, ymm1, 1 // 1 means LT
        vblendvps   ymm1, ymm12, ymm1, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm2, ymm15
        vcmpps     ymm13, ymm14, ymm2, 1 // 1 means LT
        vblendvps   ymm2, ymm12, ymm2, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm3, ymm15
        vcmpps     ymm13, ymm14, ymm3, 1 // 1 means LT
        vblendvps   ymm3, ymm12, ymm3, ymm13
    
    // select muled of orginal

    jmp    .Lnon_linear_loop

.Lq_scale:
.Lq_shl:
.Lq_shr:
    jmp .Lunsupported


// vim: set syntax=asm :

// vim: set syntax=asm :

.Lper_row_min:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm4,  [rax + 0]
    
        vmovups         ymm5,  [rax + 32]
    
        vmovups         ymm6,  [rax + 64]
    
        vmovups         ymm7,  [rax + 96]
    



    
        vminps ymm0, ymm4, ymm0
    
        vminps ymm1, ymm5, ymm1
    
        vminps ymm2, ymm6, ymm2
    
        vminps ymm3, ymm7, ymm3
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_max:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm4,  [rax + 0]
    
        vmovups         ymm5,  [rax + 32]
    
        vmovups         ymm6,  [rax + 64]
    
        vmovups         ymm7,  [rax + 96]
    



    
        vmaxps ymm0, ymm4, ymm0
    
        vmaxps ymm1, ymm5, ymm1
    
        vmaxps ymm2, ymm6, ymm2
    
        vmaxps ymm3, ymm7, ymm3
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_add:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm4,  [rax + 0]
    
        vmovups         ymm5,  [rax + 32]
    
        vmovups         ymm6,  [rax + 64]
    
        vmovups         ymm7,  [rax + 96]
    



    
        vaddps ymm0, ymm4, ymm0
    
        vaddps ymm1, ymm5, ymm1
    
        vaddps ymm2, ymm6, ymm2
    
        vaddps ymm3, ymm7, ymm3
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_mul:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm4,  [rax + 0]
    
        vmovups         ymm5,  [rax + 32]
    
        vmovups         ymm6,  [rax + 64]
    
        vmovups         ymm7,  [rax + 96]
    



    
        vmulps ymm0, ymm4, ymm0
    
        vmulps ymm1, ymm5, ymm1
    
        vmulps ymm2, ymm6, ymm2
    
        vmulps ymm3, ymm7, ymm3
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm4,  [rax + 0]
    
        vmovups         ymm5,  [rax + 32]
    
        vmovups         ymm6,  [rax + 64]
    
        vmovups         ymm7,  [rax + 96]
    



    
        vsubps ymm0, ymm4, ymm0
    
        vsubps ymm1, ymm5, ymm1
    
        vsubps ymm2, ymm6, ymm2
    
        vsubps ymm3, ymm7, ymm3
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub_flipped:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm4,  [rax + 0]
    
        vmovups         ymm5,  [rax + 32]
    
        vmovups         ymm6,  [rax + 64]
    
        vmovups         ymm7,  [rax + 96]
    



    
        vsubps ymm0, ymm0, ymm4
    
        vsubps ymm1, ymm1, ymm5
    
        vsubps ymm2, ymm2, ymm6
    
        vsubps ymm3, ymm3, ymm7
    


    jmp .Lnon_linear_loop



// vim: set syntax=asm :

// vim: set syntax=asm :

.Lper_col_min:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm4, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vminps ymm0, ymm4, ymm0
        
    
        
        
            vminps ymm1, ymm4, ymm1
        
    
        
        
            vminps ymm2, ymm4, ymm2
        
    
        
        
            vminps ymm3, ymm4, ymm3
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_max:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm4, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmaxps ymm0, ymm4, ymm0
        
    
        
        
            vmaxps ymm1, ymm4, ymm1
        
    
        
        
            vmaxps ymm2, ymm4, ymm2
        
    
        
        
            vmaxps ymm3, ymm4, ymm3
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_add:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm4, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vaddps ymm0, ymm4, ymm0
        
    
        
        
            vaddps ymm1, ymm4, ymm1
        
    
        
        
            vaddps ymm2, ymm4, ymm2
        
    
        
        
            vaddps ymm3, ymm4, ymm3
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_mul:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm4, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmulps ymm0, ymm4, ymm0
        
    
        
        
            vmulps ymm1, ymm4, ymm1
        
    
        
        
            vmulps ymm2, ymm4, ymm2
        
    
        
        
            vmulps ymm3, ymm4, ymm3
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm4, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm0, ymm4, ymm0
        
    
        
        
            vsubps ymm1, ymm4, ymm1
        
    
        
        
            vsubps ymm2, ymm4, ymm2
        
    
        
        
            vsubps ymm3, ymm4, ymm3
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub_flipped:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm4, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm0, ymm0, ymm4
        
    
        
        
            vsubps ymm1, ymm1, ymm4
        
    
        
        
            vsubps ymm2, ymm2, ymm4
        
    
        
        
            vsubps ymm3, ymm3, ymm4
        
    


    jmp .Lnon_linear_loop



// vim: set syntax=asm :

.Lload_tile:
    mov          r8, [rdi + 8]
    
        vmovups         ymm0, ymmword ptr [r8 + 0]
    
        vmovups         ymm1, ymmword ptr [r8 + 32]
    
        vmovups         ymm2, ymmword ptr [r8 + 64]
    
        vmovups         ymm3, ymmword ptr [r8 + 96]
    

    jmp    .Lnon_linear_loop


.Ladd_unicast:
    mov     r10,    [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride

	cmp rsi, 4
	jne .Ladd_unicast_generic

    
        vaddps ymm0, ymm0, [ r10 + 0 ]
    
        vaddps ymm1, ymm1, [ r10 + 32 ]
    
        vaddps ymm2, ymm2, [ r10 + 64 ]
    
        vaddps ymm3, ymm3, [ r10 + 96 ]
    
    jmp    .Lnon_linear_loop


    jmp    .Lnon_linear_loop

.Ladd_unicast_generic:
    mov     eax,    0

    pinsrd  xmm14, eax, 0
    add     eax,    esi

    pinsrd  xmm14, eax, 1
    add     eax,    esi

    pinsrd  xmm14, eax, 2
    add     eax,    esi

    pinsrd  xmm14, eax, 3
    add     eax,    esi


    pinsrd  xmm15, eax, 0
    add     eax,    esi

    pinsrd  xmm15, eax, 1
    add     eax,    esi

    pinsrd  xmm15, eax, 2
    add     eax,    esi

    pinsrd  xmm15, eax, 3
    add     eax,    esi


    vperm2f128      ymm14,  ymm14, ymm15,         32 // ymm14 <- xmm14::xmm15


    vpcmpeqd        ymm15,  ymm15, ymm15
    vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15

    vaddps          ymm0,   ymm0,   ymm12
    lea             r10, [ r10 + rsi * 8 ]

    vpcmpeqd        ymm15,  ymm15, ymm15
    vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15

    vaddps          ymm1,   ymm1,   ymm12
    lea             r10, [ r10 + rsi * 8 ]

    vpcmpeqd        ymm15,  ymm15, ymm15
    vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15

    vaddps          ymm2,   ymm2,   ymm12
    lea             r10, [ r10 + rsi * 8 ]

    vpcmpeqd        ymm15,  ymm15, ymm15
    vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15

    vaddps          ymm3,   ymm3,   ymm12
    lea             r10, [ r10 + rsi * 8 ]


    jmp    .Lnon_linear_loop

.Ladd_row_col_products:
    mov             rax, [ rdi + 8 ]
    mov             rbx, [ rdi + 16 ]

    vbroadcastss    ymm14, dword ptr [rbx]


    vmovups         ymm12,  [rax + 0]
    vfmadd231ps     ymm0, ymm12, ymm14

    vmovups         ymm12,  [rax + 32]
    vfmadd231ps     ymm1, ymm12, ymm14

    vmovups         ymm12,  [rax + 64]
    vfmadd231ps     ymm2, ymm12, ymm14

    vmovups         ymm12,  [rax + 96]
    vfmadd231ps     ymm3, ymm12, ymm14

    jmp    .Lnon_linear_loop

.Lstore:
    mov     r8,     [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride
    mov     r11,    [rdi + 32]          // item size

    cmp     r11, 2
    je      .Lstore_f16

	cmp rsi, 4
	jne .Lstore_generic

	
        vmovups [r8 + 0], ymm0
    
        vmovups [r8 + 32], ymm1
    
        vmovups [r8 + 64], ymm2
    
        vmovups [r8 + 96], ymm3
    

    jmp     .Lnon_linear_loop

.Lstore_generic:

    
        
            
                movaps xmm9, xmm0
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
            
                vperm2f128 ymm9, ymm0, ymm0, 1
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
    
        
            
                movaps xmm9, xmm1
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
            
                vperm2f128 ymm9, ymm1, ymm1, 1
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
    
        
            
                movaps xmm9, xmm2
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
            
                vperm2f128 ymm9, ymm2, ymm2, 1
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
    
        
            
                movaps xmm9, xmm3
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
            
                vperm2f128 ymm9, ymm3, ymm3, 1
            
            
                vextractps  dword ptr [r8], xmm9, 0
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 1
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 2
                add         r8, rsi
            
                vextractps  dword ptr [r8], xmm9, 3
                add         r8, rsi
            
        
    

    jmp    .Lnon_linear_loop

.Lstore_f16:

    vcvtps2ph   xmm0, ymm0, 0
    vcvtps2ph   xmm1, ymm1, 0
    vcvtps2ph   xmm2, ymm2, 0
    vcvtps2ph   xmm3, ymm3, 0

    cmp         rsi, 2
	jne .Lstore_generic_f16

	
        vmovups [r8 + 0], xmm0
    
        vmovups [r8 + 16], xmm1
    
        vmovups [r8 + 32], xmm2
    
        vmovups [r8 + 48], xmm3
    

    jmp     .Lnon_linear_loop
    
.Lstore_generic_f16:

    
        
            pextrw      word ptr [r8], xmm0, 0
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm0, 1
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm0, 2
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm0, 3
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm0, 4
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm0, 5
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm0, 6
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm0, 7
            add         r8, rsi
        
    
        
            pextrw      word ptr [r8], xmm1, 0
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm1, 1
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm1, 2
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm1, 3
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm1, 4
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm1, 5
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm1, 6
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm1, 7
            add         r8, rsi
        
    
        
            pextrw      word ptr [r8], xmm2, 0
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm2, 1
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm2, 2
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm2, 3
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm2, 4
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm2, 5
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm2, 6
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm2, 7
            add         r8, rsi
        
    
        
            pextrw      word ptr [r8], xmm3, 0
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm3, 1
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm3, 2
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm3, 3
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm3, 4
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm3, 5
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm3, 6
            add         r8, rsi
        
            pextrw      word ptr [r8], xmm3, 7
            add         r8, rsi
        
    

    jmp     .Lnon_linear_loop

.Lreturn:
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


.cfi_endproc


