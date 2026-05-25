




.intel_syntax noprefix
.text
.p2align 5
.globl fma_mmm_f32_24x4_0_22_1
fma_mmm_f32_24x4_0_22_1:
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
    test    rbx,    rbx
    jz      .Lnon_linear_loop

.Lmain_loop_packed_packed:
    	// Tile size: 3x4
	// Accumulators: 0-11
	// Col regs: ymm12-14
	// Row regs: ymm15

	vmovaps			ymm12,	[rax]
	vmovaps			ymm13,	[rax+32]
	vmovaps			ymm14,	[rax+64]

	vbroadcastss	ymm15,	dword ptr [rcx + 0]

	vfmadd231ps		ymm0,	ymm12, ymm15
	vfmadd231ps		ymm1,	ymm13, ymm15
	vfmadd231ps		ymm2,	ymm14, ymm15

	vbroadcastss	ymm15,	dword ptr [rcx + 4]

	vfmadd231ps		ymm3,	ymm12, ymm15
	vfmadd231ps		ymm4,	ymm13, ymm15
	vfmadd231ps		ymm5,	ymm14, ymm15

	vbroadcastss	ymm15,	dword ptr [rcx + 8]

	vfmadd231ps		ymm6,	ymm12, ymm15
	vfmadd231ps		ymm7,	ymm13, ymm15
	vfmadd231ps		ymm8,	ymm14, ymm15

	vbroadcastss	ymm15,	dword ptr [rcx + 12]

	vfmadd231ps		ymm9,	ymm12, ymm15
	vfmadd231ps		ymm10,	 ymm13, ymm15
	vfmadd231ps		ymm11,	 ymm14, ymm15


    add             rcx,    16
    add             rax,    96
    dec             rbx
    jnz             .Lmain_loop_packed_packed

    jmp             .Lnon_linear_loop

// NON LINEAR / ADDC

// vim: set syntax=asm :

// vim: set syntax=asm :

.Lscalar_min:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vminps          ymm0, ymm12, ymm0
        
            vminps          ymm1, ymm12, ymm1
        
            vminps          ymm2, ymm12, ymm2
        
            vminps          ymm3, ymm12, ymm3
        
            vminps          ymm4, ymm12, ymm4
        
            vminps          ymm5, ymm12, ymm5
        
            vminps          ymm6, ymm12, ymm6
        
            vminps          ymm7, ymm12, ymm7
        
            vminps          ymm8, ymm12, ymm8
        
            vminps          ymm9, ymm12, ymm9
        
            vminps          ymm10, ymm12, ymm10
        
            vminps          ymm11, ymm12, ymm11
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_max:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vmaxps          ymm0, ymm12, ymm0
        
            vmaxps          ymm1, ymm12, ymm1
        
            vmaxps          ymm2, ymm12, ymm2
        
            vmaxps          ymm3, ymm12, ymm3
        
            vmaxps          ymm4, ymm12, ymm4
        
            vmaxps          ymm5, ymm12, ymm5
        
            vmaxps          ymm6, ymm12, ymm6
        
            vmaxps          ymm7, ymm12, ymm7
        
            vmaxps          ymm8, ymm12, ymm8
        
            vmaxps          ymm9, ymm12, ymm9
        
            vmaxps          ymm10, ymm12, ymm10
        
            vmaxps          ymm11, ymm12, ymm11
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_add:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vaddps          ymm0, ymm12, ymm0
        
            vaddps          ymm1, ymm12, ymm1
        
            vaddps          ymm2, ymm12, ymm2
        
            vaddps          ymm3, ymm12, ymm3
        
            vaddps          ymm4, ymm12, ymm4
        
            vaddps          ymm5, ymm12, ymm5
        
            vaddps          ymm6, ymm12, ymm6
        
            vaddps          ymm7, ymm12, ymm7
        
            vaddps          ymm8, ymm12, ymm8
        
            vaddps          ymm9, ymm12, ymm9
        
            vaddps          ymm10, ymm12, ymm10
        
            vaddps          ymm11, ymm12, ymm11
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_mul:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vmulps          ymm0, ymm12, ymm0
        
            vmulps          ymm1, ymm12, ymm1
        
            vmulps          ymm2, ymm12, ymm2
        
            vmulps          ymm3, ymm12, ymm3
        
            vmulps          ymm4, ymm12, ymm4
        
            vmulps          ymm5, ymm12, ymm5
        
            vmulps          ymm6, ymm12, ymm6
        
            vmulps          ymm7, ymm12, ymm7
        
            vmulps          ymm8, ymm12, ymm8
        
            vmulps          ymm9, ymm12, ymm9
        
            vmulps          ymm10, ymm12, ymm10
        
            vmulps          ymm11, ymm12, ymm11
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vsubps          ymm0, ymm12, ymm0
        
            vsubps          ymm1, ymm12, ymm1
        
            vsubps          ymm2, ymm12, ymm2
        
            vsubps          ymm3, ymm12, ymm3
        
            vsubps          ymm4, ymm12, ymm4
        
            vsubps          ymm5, ymm12, ymm5
        
            vsubps          ymm6, ymm12, ymm6
        
            vsubps          ymm7, ymm12, ymm7
        
            vsubps          ymm8, ymm12, ymm8
        
            vsubps          ymm9, ymm12, ymm9
        
            vsubps          ymm10, ymm12, ymm10
        
            vsubps          ymm11, ymm12, ymm11
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub_flipped:
    
        vbroadcastss    ymm12, dword ptr [rdi + 8]
    
    
    
        
            vsubps          ymm0, ymm0, ymm12
        
            vsubps          ymm1, ymm1, ymm12
        
            vsubps          ymm2, ymm2, ymm12
        
            vsubps          ymm3, ymm3, ymm12
        
            vsubps          ymm4, ymm4, ymm12
        
            vsubps          ymm5, ymm5, ymm12
        
            vsubps          ymm6, ymm6, ymm12
        
            vsubps          ymm7, ymm7, ymm12
        
            vsubps          ymm8, ymm8, ymm12
        
            vsubps          ymm9, ymm9, ymm12
        
            vsubps          ymm10, ymm10, ymm12
        
            vsubps          ymm11, ymm11, ymm12
        
    

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
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm4, ymm15
        vcmpps     ymm13, ymm14, ymm4, 1 // 1 means LT
        vblendvps   ymm4, ymm12, ymm4, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm5, ymm15
        vcmpps     ymm13, ymm14, ymm5, 1 // 1 means LT
        vblendvps   ymm5, ymm12, ymm5, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm6, ymm15
        vcmpps     ymm13, ymm14, ymm6, 1 // 1 means LT
        vblendvps   ymm6, ymm12, ymm6, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm7, ymm15
        vcmpps     ymm13, ymm14, ymm7, 1 // 1 means LT
        vblendvps   ymm7, ymm12, ymm7, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm8, ymm15
        vcmpps     ymm13, ymm14, ymm8, 1 // 1 means LT
        vblendvps   ymm8, ymm12, ymm8, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm9, ymm15
        vcmpps     ymm13, ymm14, ymm9, 1 // 1 means LT
        vblendvps   ymm9, ymm12, ymm9, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm10, ymm15
        vcmpps     ymm13, ymm14, ymm10, 1 // 1 means LT
        vblendvps   ymm10, ymm12, ymm10, ymm13
    
        // ymm12 <- alpha * x
        vmulps      ymm12, ymm11, ymm15
        vcmpps     ymm13, ymm14, ymm11, 1 // 1 means LT
        vblendvps   ymm11, ymm12, ymm11, ymm13
    
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





    
        vmovups         ymm12,  [rax + 0]
    
        vmovups         ymm13,  [rax + 32]
    
        vmovups         ymm14,  [rax + 64]
    



    
        vminps ymm0, ymm12, ymm0
    
        vminps ymm1, ymm13, ymm1
    
        vminps ymm2, ymm14, ymm2
    
        vminps ymm3, ymm12, ymm3
    
        vminps ymm4, ymm13, ymm4
    
        vminps ymm5, ymm14, ymm5
    
        vminps ymm6, ymm12, ymm6
    
        vminps ymm7, ymm13, ymm7
    
        vminps ymm8, ymm14, ymm8
    
        vminps ymm9, ymm12, ymm9
    
        vminps ymm10, ymm13, ymm10
    
        vminps ymm11, ymm14, ymm11
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_max:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm12,  [rax + 0]
    
        vmovups         ymm13,  [rax + 32]
    
        vmovups         ymm14,  [rax + 64]
    



    
        vmaxps ymm0, ymm12, ymm0
    
        vmaxps ymm1, ymm13, ymm1
    
        vmaxps ymm2, ymm14, ymm2
    
        vmaxps ymm3, ymm12, ymm3
    
        vmaxps ymm4, ymm13, ymm4
    
        vmaxps ymm5, ymm14, ymm5
    
        vmaxps ymm6, ymm12, ymm6
    
        vmaxps ymm7, ymm13, ymm7
    
        vmaxps ymm8, ymm14, ymm8
    
        vmaxps ymm9, ymm12, ymm9
    
        vmaxps ymm10, ymm13, ymm10
    
        vmaxps ymm11, ymm14, ymm11
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_add:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm12,  [rax + 0]
    
        vmovups         ymm13,  [rax + 32]
    
        vmovups         ymm14,  [rax + 64]
    



    
        vaddps ymm0, ymm12, ymm0
    
        vaddps ymm1, ymm13, ymm1
    
        vaddps ymm2, ymm14, ymm2
    
        vaddps ymm3, ymm12, ymm3
    
        vaddps ymm4, ymm13, ymm4
    
        vaddps ymm5, ymm14, ymm5
    
        vaddps ymm6, ymm12, ymm6
    
        vaddps ymm7, ymm13, ymm7
    
        vaddps ymm8, ymm14, ymm8
    
        vaddps ymm9, ymm12, ymm9
    
        vaddps ymm10, ymm13, ymm10
    
        vaddps ymm11, ymm14, ymm11
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_mul:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm12,  [rax + 0]
    
        vmovups         ymm13,  [rax + 32]
    
        vmovups         ymm14,  [rax + 64]
    



    
        vmulps ymm0, ymm12, ymm0
    
        vmulps ymm1, ymm13, ymm1
    
        vmulps ymm2, ymm14, ymm2
    
        vmulps ymm3, ymm12, ymm3
    
        vmulps ymm4, ymm13, ymm4
    
        vmulps ymm5, ymm14, ymm5
    
        vmulps ymm6, ymm12, ymm6
    
        vmulps ymm7, ymm13, ymm7
    
        vmulps ymm8, ymm14, ymm8
    
        vmulps ymm9, ymm12, ymm9
    
        vmulps ymm10, ymm13, ymm10
    
        vmulps ymm11, ymm14, ymm11
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm12,  [rax + 0]
    
        vmovups         ymm13,  [rax + 32]
    
        vmovups         ymm14,  [rax + 64]
    



    
        vsubps ymm0, ymm12, ymm0
    
        vsubps ymm1, ymm13, ymm1
    
        vsubps ymm2, ymm14, ymm2
    
        vsubps ymm3, ymm12, ymm3
    
        vsubps ymm4, ymm13, ymm4
    
        vsubps ymm5, ymm14, ymm5
    
        vsubps ymm6, ymm12, ymm6
    
        vsubps ymm7, ymm13, ymm7
    
        vsubps ymm8, ymm14, ymm8
    
        vsubps ymm9, ymm12, ymm9
    
        vsubps ymm10, ymm13, ymm10
    
        vsubps ymm11, ymm14, ymm11
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub_flipped:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm12,  [rax + 0]
    
        vmovups         ymm13,  [rax + 32]
    
        vmovups         ymm14,  [rax + 64]
    



    
        vsubps ymm0, ymm0, ymm12
    
        vsubps ymm1, ymm1, ymm13
    
        vsubps ymm2, ymm2, ymm14
    
        vsubps ymm3, ymm3, ymm12
    
        vsubps ymm4, ymm4, ymm13
    
        vsubps ymm5, ymm5, ymm14
    
        vsubps ymm6, ymm6, ymm12
    
        vsubps ymm7, ymm7, ymm13
    
        vsubps ymm8, ymm8, ymm14
    
        vsubps ymm9, ymm9, ymm12
    
        vsubps ymm10, ymm10, ymm13
    
        vsubps ymm11, ymm11, ymm14
    


    jmp .Lnon_linear_loop



// vim: set syntax=asm :

// vim: set syntax=asm :

.Lper_col_min:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vminps ymm0, ymm12, ymm0
        
    
        
        
            vminps ymm1, ymm12, ymm1
        
    
        
        
            vminps ymm2, ymm12, ymm2
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vminps ymm3, ymm12, ymm3
        
    
        
        
            vminps ymm4, ymm12, ymm4
        
    
        
        
            vminps ymm5, ymm12, ymm5
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vminps ymm6, ymm12, ymm6
        
    
        
        
            vminps ymm7, ymm12, ymm7
        
    
        
        
            vminps ymm8, ymm12, ymm8
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vminps ymm9, ymm12, ymm9
        
    
        
        
            vminps ymm10, ymm12, ymm10
        
    
        
        
            vminps ymm11, ymm12, ymm11
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_max:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmaxps ymm0, ymm12, ymm0
        
    
        
        
            vmaxps ymm1, ymm12, ymm1
        
    
        
        
            vmaxps ymm2, ymm12, ymm2
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmaxps ymm3, ymm12, ymm3
        
    
        
        
            vmaxps ymm4, ymm12, ymm4
        
    
        
        
            vmaxps ymm5, ymm12, ymm5
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmaxps ymm6, ymm12, ymm6
        
    
        
        
            vmaxps ymm7, ymm12, ymm7
        
    
        
        
            vmaxps ymm8, ymm12, ymm8
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmaxps ymm9, ymm12, ymm9
        
    
        
        
            vmaxps ymm10, ymm12, ymm10
        
    
        
        
            vmaxps ymm11, ymm12, ymm11
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_add:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vaddps ymm0, ymm12, ymm0
        
    
        
        
            vaddps ymm1, ymm12, ymm1
        
    
        
        
            vaddps ymm2, ymm12, ymm2
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vaddps ymm3, ymm12, ymm3
        
    
        
        
            vaddps ymm4, ymm12, ymm4
        
    
        
        
            vaddps ymm5, ymm12, ymm5
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vaddps ymm6, ymm12, ymm6
        
    
        
        
            vaddps ymm7, ymm12, ymm7
        
    
        
        
            vaddps ymm8, ymm12, ymm8
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vaddps ymm9, ymm12, ymm9
        
    
        
        
            vaddps ymm10, ymm12, ymm10
        
    
        
        
            vaddps ymm11, ymm12, ymm11
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_mul:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmulps ymm0, ymm12, ymm0
        
    
        
        
            vmulps ymm1, ymm12, ymm1
        
    
        
        
            vmulps ymm2, ymm12, ymm2
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmulps ymm3, ymm12, ymm3
        
    
        
        
            vmulps ymm4, ymm12, ymm4
        
    
        
        
            vmulps ymm5, ymm12, ymm5
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmulps ymm6, ymm12, ymm6
        
    
        
        
            vmulps ymm7, ymm12, ymm7
        
    
        
        
            vmulps ymm8, ymm12, ymm8
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmulps ymm9, ymm12, ymm9
        
    
        
        
            vmulps ymm10, ymm12, ymm10
        
    
        
        
            vmulps ymm11, ymm12, ymm11
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm0, ymm12, ymm0
        
    
        
        
            vsubps ymm1, ymm12, ymm1
        
    
        
        
            vsubps ymm2, ymm12, ymm2
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm3, ymm12, ymm3
        
    
        
        
            vsubps ymm4, ymm12, ymm4
        
    
        
        
            vsubps ymm5, ymm12, ymm5
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm6, ymm12, ymm6
        
    
        
        
            vsubps ymm7, ymm12, ymm7
        
    
        
        
            vsubps ymm8, ymm12, ymm8
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm9, ymm12, ymm9
        
    
        
        
            vsubps ymm10, ymm12, ymm10
        
    
        
        
            vsubps ymm11, ymm12, ymm11
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub_flipped:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm0, ymm0, ymm12
        
    
        
        
            vsubps ymm1, ymm1, ymm12
        
    
        
        
            vsubps ymm2, ymm2, ymm12
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm3, ymm3, ymm12
        
    
        
        
            vsubps ymm4, ymm4, ymm12
        
    
        
        
            vsubps ymm5, ymm5, ymm12
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm6, ymm6, ymm12
        
    
        
        
            vsubps ymm7, ymm7, ymm12
        
    
        
        
            vsubps ymm8, ymm8, ymm12
        
    

    
        vbroadcastss    ymm12, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm9, ymm9, ymm12
        
    
        
        
            vsubps ymm10, ymm10, ymm12
        
    
        
        
            vsubps ymm11, ymm11, ymm12
        
    


    jmp .Lnon_linear_loop



// vim: set syntax=asm :

.Lload_tile:
    mov          r8, [rdi + 8]
    
        vmovups         ymm0, ymmword ptr [r8 + 0]
    
        vmovups         ymm1, ymmword ptr [r8 + 32]
    
        vmovups         ymm2, ymmword ptr [r8 + 64]
    
        vmovups         ymm3, ymmword ptr [r8 + 96]
    
        vmovups         ymm4, ymmword ptr [r8 + 128]
    
        vmovups         ymm5, ymmword ptr [r8 + 160]
    
        vmovups         ymm6, ymmword ptr [r8 + 192]
    
        vmovups         ymm7, ymmword ptr [r8 + 224]
    
        vmovups         ymm8, ymmword ptr [r8 + 256]
    
        vmovups         ymm9, ymmword ptr [r8 + 288]
    
        vmovups         ymm10, ymmword ptr [r8 + 320]
    
        vmovups         ymm11, ymmword ptr [r8 + 352]
    

    jmp    .Lnon_linear_loop


.Ladd_unicast:

    mov     r8,    [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride
    mov     rbx,    [rdi + 24]          // col stride

    cmp rsi, 4
    jne .Lunicast_generic

    lea             r9,  [ r8 + rbx ]
    lea             r10, [ r9 + rbx]
    lea             r11, [ r10 + rbx ]
    lea             r12, [ r11 + rbx ]


    
        vmovups ymm12,  [ r8 ]
        add r8, 32
        vaddps ymm0, ymm0, ymm12
    
        vmovups ymm12,  [ r8 ]
        add r8, 32
        vaddps ymm1, ymm1, ymm12
    
        vmovups ymm12,  [ r8 ]
        add r8, 32
        vaddps ymm2, ymm2, ymm12
    

    
        vmovups ymm12,  [ r9 ]
        add r9, 32
        vaddps ymm3, ymm3, ymm12
    
        vmovups ymm12,  [ r9 ]
        add r9, 32
        vaddps ymm4, ymm4, ymm12
    
        vmovups ymm12,  [ r9 ]
        add r9, 32
        vaddps ymm5, ymm5, ymm12
    

    
        vmovups ymm12,  [ r10 ]
        add r10, 32
        vaddps ymm6, ymm6, ymm12
    
        vmovups ymm12,  [ r10 ]
        add r10, 32
        vaddps ymm7, ymm7, ymm12
    
        vmovups ymm12,  [ r10 ]
        add r10, 32
        vaddps ymm8, ymm8, ymm12
    

    
        vmovups ymm12,  [ r11 ]
        add r11, 32
        vaddps ymm9, ymm9, ymm12
    
        vmovups ymm12,  [ r11 ]
        add r11, 32
        vaddps ymm10, ymm10, ymm12
    
        vmovups ymm12,  [ r11 ]
        add r11, 32
        vaddps ymm11, ymm11, ymm12
    

    jmp    .Lnon_linear_loop

.Lunicast_generic:
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


//  mov r12, [0]
    vperm2f128      ymm14,  ymm14, ymm15,         32 // ymm14 <- xmm14::xmm15

    lea             r9, [ r8 + rsi * 8 ]
    lea             r10, [ r9 + rsi * 8 ]


   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r8 + ymm14 ], ymm15
      add r8, rbx
      vaddps ymm0, ymm0, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r9 + ymm14 ], ymm15
      add r9, rbx
      vaddps ymm1, ymm1, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15
      add r10, rbx
      vaddps ymm2, ymm2, ymm12
   

   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r8 + ymm14 ], ymm15
      add r8, rbx
      vaddps ymm3, ymm3, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r9 + ymm14 ], ymm15
      add r9, rbx
      vaddps ymm4, ymm4, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15
      add r10, rbx
      vaddps ymm5, ymm5, ymm12
   

   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r8 + ymm14 ], ymm15
      add r8, rbx
      vaddps ymm6, ymm6, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r9 + ymm14 ], ymm15
      add r9, rbx
      vaddps ymm7, ymm7, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15
      add r10, rbx
      vaddps ymm8, ymm8, ymm12
   

   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r8 + ymm14 ], ymm15
      add r8, rbx
      vaddps ymm9, ymm9, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r9 + ymm14 ], ymm15
      add r9, rbx
      vaddps ymm10, ymm10, ymm12
   
      vpcmpeqd        ymm15,  ymm15, ymm15
      vgatherdps      ymm12,  [ r10 + ymm14 ], ymm15
      add r10, rbx
      vaddps ymm11, ymm11, ymm12
   


    jmp    .Lnon_linear_loop

.Ladd_row_col_products:
    mov             rax, [ rdi + 8 ]
    mov             rbx, [ rdi + 16 ]

    vmovups         ymm12,  [rax]
    vmovups         ymm13,  [rax + 32]
    vmovups         ymm15,  [rax + 64]

    vbroadcastss    ymm14, dword ptr [rbx + 0 ]
    vfmadd231ps     ymm0,   ymm12, ymm14
    vfmadd231ps     ymm1, ymm13, ymm14
    vfmadd231ps     ymm2, ymm15, ymm14

    vbroadcastss    ymm14, dword ptr [rbx + 4 ]
    vfmadd231ps     ymm3,   ymm12, ymm14
    vfmadd231ps     ymm4, ymm13, ymm14
    vfmadd231ps     ymm5, ymm15, ymm14

    vbroadcastss    ymm14, dword ptr [rbx + 8 ]
    vfmadd231ps     ymm6,   ymm12, ymm14
    vfmadd231ps     ymm7, ymm13, ymm14
    vfmadd231ps     ymm8, ymm15, ymm14

    vbroadcastss    ymm14, dword ptr [rbx + 12 ]
    vfmadd231ps     ymm9,   ymm12, ymm14
    vfmadd231ps     ymm10, ymm13, ymm14
    vfmadd231ps     ymm11, ymm15, ymm14


    jmp    .Lnon_linear_loop

.Lstore:
    mov     r8,     [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride
    mov     rbx,    [rdi + 24]          // col stride

    lea     r9,     [ r8  +     rbx ]
    lea     r10,    [ r8  + 2 * rbx ]
    lea     r11,    [ r10 +     rbx ]

    cmp         rsi, 4
    jne         .Lstore_strides_generic

    
       
            vmovups ymmword ptr [r8], ymm0
            add r8, 32
       
            vmovups ymmword ptr [r8], ymm1
            add r8, 32
       
            vmovups ymmword ptr [r8], ymm2
            add r8, 32
       
    
       
            vmovups ymmword ptr [r9], ymm3
            add r9, 32
       
            vmovups ymmword ptr [r9], ymm4
            add r9, 32
       
            vmovups ymmword ptr [r9], ymm5
            add r9, 32
       
    
       
            vmovups ymmword ptr [r10], ymm6
            add r10, 32
       
            vmovups ymmword ptr [r10], ymm7
            add r10, 32
       
            vmovups ymmword ptr [r10], ymm8
            add r10, 32
       
    
       
            vmovups ymmword ptr [r11], ymm9
            add r11, 32
       
            vmovups ymmword ptr [r11], ymm10
            add r11, 32
       
            vmovups ymmword ptr [r11], ymm11
            add r11, 32
       
    

    jmp     .Lnon_linear_loop

.Lstore_strides_generic:
    
       
           
                vextractps  dword ptr [r8], xmm0, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm0, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm0, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm0, 3
                add         r8, rsi
           
           vperm2f128  ymm0, ymm0, ymm0, 1
           
                vextractps  dword ptr [r8], xmm0, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm0, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm0, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm0, 3
                add         r8, rsi
           
       
           
                vextractps  dword ptr [r8], xmm1, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm1, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm1, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm1, 3
                add         r8, rsi
           
           vperm2f128  ymm1, ymm1, ymm1, 1
           
                vextractps  dword ptr [r8], xmm1, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm1, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm1, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm1, 3
                add         r8, rsi
           
       
           
                vextractps  dword ptr [r8], xmm2, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm2, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm2, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm2, 3
                add         r8, rsi
           
           vperm2f128  ymm2, ymm2, ymm2, 1
           
                vextractps  dword ptr [r8], xmm2, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm2, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm2, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm2, 3
                add         r8, rsi
           
       
    
       
           
                vextractps  dword ptr [r9], xmm3, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm3, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm3, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm3, 3
                add         r9, rsi
           
           vperm2f128  ymm3, ymm3, ymm3, 1
           
                vextractps  dword ptr [r9], xmm3, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm3, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm3, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm3, 3
                add         r9, rsi
           
       
           
                vextractps  dword ptr [r9], xmm4, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm4, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm4, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm4, 3
                add         r9, rsi
           
           vperm2f128  ymm4, ymm4, ymm4, 1
           
                vextractps  dword ptr [r9], xmm4, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm4, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm4, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm4, 3
                add         r9, rsi
           
       
           
                vextractps  dword ptr [r9], xmm5, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm5, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm5, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm5, 3
                add         r9, rsi
           
           vperm2f128  ymm5, ymm5, ymm5, 1
           
                vextractps  dword ptr [r9], xmm5, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm5, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm5, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm5, 3
                add         r9, rsi
           
       
    
       
           
                vextractps  dword ptr [r10], xmm6, 0
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm6, 1
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm6, 2
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm6, 3
                add         r10, rsi
           
           vperm2f128  ymm6, ymm6, ymm6, 1
           
                vextractps  dword ptr [r10], xmm6, 0
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm6, 1
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm6, 2
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm6, 3
                add         r10, rsi
           
       
           
                vextractps  dword ptr [r10], xmm7, 0
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm7, 1
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm7, 2
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm7, 3
                add         r10, rsi
           
           vperm2f128  ymm7, ymm7, ymm7, 1
           
                vextractps  dword ptr [r10], xmm7, 0
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm7, 1
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm7, 2
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm7, 3
                add         r10, rsi
           
       
           
                vextractps  dword ptr [r10], xmm8, 0
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm8, 1
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm8, 2
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm8, 3
                add         r10, rsi
           
           vperm2f128  ymm8, ymm8, ymm8, 1
           
                vextractps  dword ptr [r10], xmm8, 0
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm8, 1
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm8, 2
                add         r10, rsi
           
                vextractps  dword ptr [r10], xmm8, 3
                add         r10, rsi
           
       
    
       
           
                vextractps  dword ptr [r11], xmm9, 0
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm9, 1
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm9, 2
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm9, 3
                add         r11, rsi
           
           vperm2f128  ymm9, ymm9, ymm9, 1
           
                vextractps  dword ptr [r11], xmm9, 0
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm9, 1
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm9, 2
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm9, 3
                add         r11, rsi
           
       
           
                vextractps  dword ptr [r11], xmm10, 0
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm10, 1
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm10, 2
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm10, 3
                add         r11, rsi
           
           vperm2f128  ymm10, ymm10, ymm10, 1
           
                vextractps  dword ptr [r11], xmm10, 0
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm10, 1
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm10, 2
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm10, 3
                add         r11, rsi
           
       
           
                vextractps  dword ptr [r11], xmm11, 0
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm11, 1
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm11, 2
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm11, 3
                add         r11, rsi
           
           vperm2f128  ymm11, ymm11, ymm11, 1
           
                vextractps  dword ptr [r11], xmm11, 0
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm11, 1
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm11, 2
                add         r11, rsi
           
                vextractps  dword ptr [r11], xmm11, 3
                add         r11, rsi
           
       
    
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


