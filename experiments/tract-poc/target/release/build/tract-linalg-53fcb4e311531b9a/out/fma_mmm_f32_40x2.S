




.intel_syntax noprefix
.text
.p2align 5
.globl fma_mmm_f32_40x2_0_22_1
fma_mmm_f32_40x2_0_22_1:
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
    	// Tile size: 5x2
	// Accumulators: 0-9
	// Col regs: ymm10-13
	// Row regs: ymm14-15

	vmovaps			ymm10,	[rax]
	vbroadcastss	ymm14,	dword ptr [rcx + 0]
	vbroadcastss	ymm15,	dword ptr [rcx + 4]
	vmovaps			ymm11,	[rax + 32]

	// NB stepping column-wise
	vfmadd231ps		ymm0,	ymm10, ymm14
	vfmadd231ps		ymm5,	ymm10, ymm15

	vmovaps			ymm12,	[rax + 64]

	vfmadd231ps		ymm1,	ymm11, ymm14
	vfmadd231ps		ymm6,	ymm11, ymm15

	vmovaps			ymm13,	[rax + 96]

	vfmadd231ps		ymm2,	ymm12, ymm14
	vfmadd231ps		ymm7,	ymm12, ymm15

	vmovaps			ymm11,	[rax + 128]

	vfmadd231ps		ymm3,	ymm13, ymm14
	vfmadd231ps		ymm8,	ymm13, ymm15

	vfmadd231ps		ymm4,	ymm11, ymm14
	vfmadd231ps		ymm9,	ymm11, ymm15

	add rax, 160
	add rcx, 8


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





    
        vmovups         ymm10,  [rax + 0]
    
        vmovups         ymm11,  [rax + 32]
    
        vmovups         ymm12,  [rax + 64]
    
        vmovups         ymm13,  [rax + 96]
    
        vmovups         ymm14,  [rax + 128]
    



    
        vminps ymm0, ymm10, ymm0
    
        vminps ymm1, ymm11, ymm1
    
        vminps ymm2, ymm12, ymm2
    
        vminps ymm3, ymm13, ymm3
    
        vminps ymm4, ymm14, ymm4
    
        vminps ymm5, ymm10, ymm5
    
        vminps ymm6, ymm11, ymm6
    
        vminps ymm7, ymm12, ymm7
    
        vminps ymm8, ymm13, ymm8
    
        vminps ymm9, ymm14, ymm9
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_max:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm10,  [rax + 0]
    
        vmovups         ymm11,  [rax + 32]
    
        vmovups         ymm12,  [rax + 64]
    
        vmovups         ymm13,  [rax + 96]
    
        vmovups         ymm14,  [rax + 128]
    



    
        vmaxps ymm0, ymm10, ymm0
    
        vmaxps ymm1, ymm11, ymm1
    
        vmaxps ymm2, ymm12, ymm2
    
        vmaxps ymm3, ymm13, ymm3
    
        vmaxps ymm4, ymm14, ymm4
    
        vmaxps ymm5, ymm10, ymm5
    
        vmaxps ymm6, ymm11, ymm6
    
        vmaxps ymm7, ymm12, ymm7
    
        vmaxps ymm8, ymm13, ymm8
    
        vmaxps ymm9, ymm14, ymm9
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_add:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm10,  [rax + 0]
    
        vmovups         ymm11,  [rax + 32]
    
        vmovups         ymm12,  [rax + 64]
    
        vmovups         ymm13,  [rax + 96]
    
        vmovups         ymm14,  [rax + 128]
    



    
        vaddps ymm0, ymm10, ymm0
    
        vaddps ymm1, ymm11, ymm1
    
        vaddps ymm2, ymm12, ymm2
    
        vaddps ymm3, ymm13, ymm3
    
        vaddps ymm4, ymm14, ymm4
    
        vaddps ymm5, ymm10, ymm5
    
        vaddps ymm6, ymm11, ymm6
    
        vaddps ymm7, ymm12, ymm7
    
        vaddps ymm8, ymm13, ymm8
    
        vaddps ymm9, ymm14, ymm9
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_mul:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm10,  [rax + 0]
    
        vmovups         ymm11,  [rax + 32]
    
        vmovups         ymm12,  [rax + 64]
    
        vmovups         ymm13,  [rax + 96]
    
        vmovups         ymm14,  [rax + 128]
    



    
        vmulps ymm0, ymm10, ymm0
    
        vmulps ymm1, ymm11, ymm1
    
        vmulps ymm2, ymm12, ymm2
    
        vmulps ymm3, ymm13, ymm3
    
        vmulps ymm4, ymm14, ymm4
    
        vmulps ymm5, ymm10, ymm5
    
        vmulps ymm6, ymm11, ymm6
    
        vmulps ymm7, ymm12, ymm7
    
        vmulps ymm8, ymm13, ymm8
    
        vmulps ymm9, ymm14, ymm9
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm10,  [rax + 0]
    
        vmovups         ymm11,  [rax + 32]
    
        vmovups         ymm12,  [rax + 64]
    
        vmovups         ymm13,  [rax + 96]
    
        vmovups         ymm14,  [rax + 128]
    



    
        vsubps ymm0, ymm10, ymm0
    
        vsubps ymm1, ymm11, ymm1
    
        vsubps ymm2, ymm12, ymm2
    
        vsubps ymm3, ymm13, ymm3
    
        vsubps ymm4, ymm14, ymm4
    
        vsubps ymm5, ymm10, ymm5
    
        vsubps ymm6, ymm11, ymm6
    
        vsubps ymm7, ymm12, ymm7
    
        vsubps ymm8, ymm13, ymm8
    
        vsubps ymm9, ymm14, ymm9
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub_flipped:
    mov             rax, [ rdi + 8 ]





    
        vmovups         ymm10,  [rax + 0]
    
        vmovups         ymm11,  [rax + 32]
    
        vmovups         ymm12,  [rax + 64]
    
        vmovups         ymm13,  [rax + 96]
    
        vmovups         ymm14,  [rax + 128]
    



    
        vsubps ymm0, ymm0, ymm10
    
        vsubps ymm1, ymm1, ymm11
    
        vsubps ymm2, ymm2, ymm12
    
        vsubps ymm3, ymm3, ymm13
    
        vsubps ymm4, ymm4, ymm14
    
        vsubps ymm5, ymm5, ymm10
    
        vsubps ymm6, ymm6, ymm11
    
        vsubps ymm7, ymm7, ymm12
    
        vsubps ymm8, ymm8, ymm13
    
        vsubps ymm9, ymm9, ymm14
    


    jmp .Lnon_linear_loop



// vim: set syntax=asm :

// vim: set syntax=asm :

.Lper_col_min:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vminps ymm0, ymm10, ymm0
        
    
        
        
            vminps ymm1, ymm10, ymm1
        
    
        
        
            vminps ymm2, ymm10, ymm2
        
    
        
        
            vminps ymm3, ymm10, ymm3
        
    
        
        
            vminps ymm4, ymm10, ymm4
        
    

    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vminps ymm5, ymm10, ymm5
        
    
        
        
            vminps ymm6, ymm10, ymm6
        
    
        
        
            vminps ymm7, ymm10, ymm7
        
    
        
        
            vminps ymm8, ymm10, ymm8
        
    
        
        
            vminps ymm9, ymm10, ymm9
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_max:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmaxps ymm0, ymm10, ymm0
        
    
        
        
            vmaxps ymm1, ymm10, ymm1
        
    
        
        
            vmaxps ymm2, ymm10, ymm2
        
    
        
        
            vmaxps ymm3, ymm10, ymm3
        
    
        
        
            vmaxps ymm4, ymm10, ymm4
        
    

    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmaxps ymm5, ymm10, ymm5
        
    
        
        
            vmaxps ymm6, ymm10, ymm6
        
    
        
        
            vmaxps ymm7, ymm10, ymm7
        
    
        
        
            vmaxps ymm8, ymm10, ymm8
        
    
        
        
            vmaxps ymm9, ymm10, ymm9
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_add:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vaddps ymm0, ymm10, ymm0
        
    
        
        
            vaddps ymm1, ymm10, ymm1
        
    
        
        
            vaddps ymm2, ymm10, ymm2
        
    
        
        
            vaddps ymm3, ymm10, ymm3
        
    
        
        
            vaddps ymm4, ymm10, ymm4
        
    

    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vaddps ymm5, ymm10, ymm5
        
    
        
        
            vaddps ymm6, ymm10, ymm6
        
    
        
        
            vaddps ymm7, ymm10, ymm7
        
    
        
        
            vaddps ymm8, ymm10, ymm8
        
    
        
        
            vaddps ymm9, ymm10, ymm9
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_mul:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmulps ymm0, ymm10, ymm0
        
    
        
        
            vmulps ymm1, ymm10, ymm1
        
    
        
        
            vmulps ymm2, ymm10, ymm2
        
    
        
        
            vmulps ymm3, ymm10, ymm3
        
    
        
        
            vmulps ymm4, ymm10, ymm4
        
    

    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vmulps ymm5, ymm10, ymm5
        
    
        
        
            vmulps ymm6, ymm10, ymm6
        
    
        
        
            vmulps ymm7, ymm10, ymm7
        
    
        
        
            vmulps ymm8, ymm10, ymm8
        
    
        
        
            vmulps ymm9, ymm10, ymm9
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm0, ymm10, ymm0
        
    
        
        
            vsubps ymm1, ymm10, ymm1
        
    
        
        
            vsubps ymm2, ymm10, ymm2
        
    
        
        
            vsubps ymm3, ymm10, ymm3
        
    
        
        
            vsubps ymm4, ymm10, ymm4
        
    

    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm5, ymm10, ymm5
        
    
        
        
            vsubps ymm6, ymm10, ymm6
        
    
        
        
            vsubps ymm7, ymm10, ymm7
        
    
        
        
            vsubps ymm8, ymm10, ymm8
        
    
        
        
            vsubps ymm9, ymm10, ymm9
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub_flipped:
    mov             rax, [ rdi + 8 ]











    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm0, ymm0, ymm10
        
    
        
        
            vsubps ymm1, ymm1, ymm10
        
    
        
        
            vsubps ymm2, ymm2, ymm10
        
    
        
        
            vsubps ymm3, ymm3, ymm10
        
    
        
        
            vsubps ymm4, ymm4, ymm10
        
    

    
        vbroadcastss    ymm10, dword ptr [ rax ]
        add             rax, 4
    
    
        
        
            vsubps ymm5, ymm5, ymm10
        
    
        
        
            vsubps ymm6, ymm6, ymm10
        
    
        
        
            vsubps ymm7, ymm7, ymm10
        
    
        
        
            vsubps ymm8, ymm8, ymm10
        
    
        
        
            vsubps ymm9, ymm9, ymm10
        
    


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
        add		r8, 32
        vaddps 	ymm0, ymm0, ymm12
    
        vmovups ymm12,  [ r8 ]
        add		r8, 32
        vaddps 	ymm1, ymm1, ymm12
    
        vmovups ymm12,  [ r8 ]
        add		r8, 32
        vaddps 	ymm2, ymm2, ymm12
    
        vmovups ymm12,  [ r8 ]
        add		r8, 32
        vaddps 	ymm3, ymm3, ymm12
    
        vmovups ymm12,  [ r8 ]
        add		r8, 32
        vaddps 	ymm4, ymm4, ymm12
    

    
        vmovups ymm12,  [ r9 ]
        add		r9, 32
        vaddps 	ymm5, ymm5, ymm12
    
        vmovups ymm12,  [ r9 ]
        add		r9, 32
        vaddps 	ymm6, ymm6, ymm12
    
        vmovups ymm12,  [ r9 ]
        add		r9, 32
        vaddps 	ymm7, ymm7, ymm12
    
        vmovups ymm12,  [ r9 ]
        add		r9, 32
        vaddps 	ymm8, ymm8, ymm12
    
        vmovups ymm12,  [ r9 ]
        add		r9, 32
        vaddps 	ymm9, ymm9, ymm12
    

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


    vperm2f128      ymm14,  ymm14, ymm15,         32 // ymm14 <- xmm14::xmm15

    lea             r9,  [ r8 + rsi * 8]
    lea             r10, [ r9 + rsi * 8]
    lea             r11, [ r10 + rsi * 8]
    lea             r12, [ r11 + rsi * 8]


   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r8 + ymm14 ], ymm15
      add 			  r8, 	rbx
      vaddps 		  ymm0, ymm0, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r9 + ymm14 ], ymm15
      add 			  r9, 	rbx
      vaddps 		  ymm1, ymm1, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r10 + ymm14 ], ymm15
      add 			  r10, 	rbx
      vaddps 		  ymm2, ymm2, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r11 + ymm14 ], ymm15
      add 			  r11, 	rbx
      vaddps 		  ymm3, ymm3, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r12 + ymm14 ], ymm15
      add 			  r12, 	rbx
      vaddps 		  ymm4, ymm4, ymm12
   

   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r8 + ymm14 ], ymm15
      add 			  r8, 	rbx
      vaddps 		  ymm5, ymm5, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r9 + ymm14 ], ymm15
      add 			  r9, 	rbx
      vaddps 		  ymm6, ymm6, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r10 + ymm14 ], ymm15
      add 			  r10, 	rbx
      vaddps 		  ymm7, ymm7, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r11 + ymm14 ], ymm15
      add 			  r11, 	rbx
      vaddps 		  ymm8, ymm8, ymm12
   
      vpcmpeqd        ymm15, ymm15, ymm15
      vgatherdps      ymm12, [ r12 + ymm14 ], ymm15
      add 			  r12, 	rbx
      vaddps 		  ymm9, ymm9, ymm12
   


    jmp    .Lnon_linear_loop

.Ladd_row_col_products:
    mov             rax, [ rdi + 8 ]
    mov             rbx, [ rdi + 16 ]

    vbroadcastss    ymm10, dword ptr [rbx]
    vbroadcastss    ymm11, dword ptr [rbx + 4]

    vmovups         ymm12,  [rax + 0]
    vfmadd231ps     ymm0, ymm12, ymm10
    vfmadd231ps     ymm5, ymm12, ymm11

    vmovups         ymm12,  [rax + 32]
    vfmadd231ps     ymm1, ymm12, ymm10
    vfmadd231ps     ymm6, ymm12, ymm11

    vmovups         ymm12,  [rax + 64]
    vfmadd231ps     ymm2, ymm12, ymm10
    vfmadd231ps     ymm7, ymm12, ymm11

    vmovups         ymm12,  [rax + 96]
    vfmadd231ps     ymm3, ymm12, ymm10
    vfmadd231ps     ymm8, ymm12, ymm11

    vmovups         ymm12,  [rax + 128]
    vfmadd231ps     ymm4, ymm12, ymm10
    vfmadd231ps     ymm9, ymm12, ymm11

    jmp    .Lnon_linear_loop


.Lstore:
    mov     r8,     [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride
    mov     rbx,    [rdi + 24]          // col stride

    lea     r9,     [ r8  +     rbx ]
    lea     r10,    [ r8  + 2 * rbx ]
    lea     r11,    [ r10 +     rbx ]
    lea     r12,    [ r10 + 2 * rbx ]

    cmp         rsi, 4
    jne         .Lstore_strides_generic

    
       
            vmovups ymmword ptr [r8], ymm0
            add 	r8, 32
       
            vmovups ymmword ptr [r8], ymm1
            add 	r8, 32
       
            vmovups ymmword ptr [r8], ymm2
            add 	r8, 32
       
            vmovups ymmword ptr [r8], ymm3
            add 	r8, 32
       
            vmovups ymmword ptr [r8], ymm4
            add 	r8, 32
       
    
       
            vmovups ymmword ptr [r9], ymm5
            add 	r9, 32
       
            vmovups ymmword ptr [r9], ymm6
            add 	r9, 32
       
            vmovups ymmword ptr [r9], ymm7
            add 	r9, 32
       
            vmovups ymmword ptr [r9], ymm8
            add 	r9, 32
       
            vmovups ymmword ptr [r9], ymm9
            add 	r9, 32
       
    

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
           
       
           
                vextractps  dword ptr [r8], xmm3, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm3, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm3, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm3, 3
                add         r8, rsi
           
           vperm2f128  ymm3, ymm3, ymm3, 1
           
                vextractps  dword ptr [r8], xmm3, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm3, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm3, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm3, 3
                add         r8, rsi
           
       
           
                vextractps  dword ptr [r8], xmm4, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm4, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm4, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm4, 3
                add         r8, rsi
           
           vperm2f128  ymm4, ymm4, ymm4, 1
           
                vextractps  dword ptr [r8], xmm4, 0
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm4, 1
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm4, 2
                add         r8, rsi
           
                vextractps  dword ptr [r8], xmm4, 3
                add         r8, rsi
           
       
    
       
           
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
           
       
           
                vextractps  dword ptr [r9], xmm6, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm6, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm6, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm6, 3
                add         r9, rsi
           
           vperm2f128  ymm6, ymm6, ymm6, 1
           
                vextractps  dword ptr [r9], xmm6, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm6, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm6, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm6, 3
                add         r9, rsi
           
       
           
                vextractps  dword ptr [r9], xmm7, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm7, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm7, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm7, 3
                add         r9, rsi
           
           vperm2f128  ymm7, ymm7, ymm7, 1
           
                vextractps  dword ptr [r9], xmm7, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm7, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm7, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm7, 3
                add         r9, rsi
           
       
           
                vextractps  dword ptr [r9], xmm8, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm8, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm8, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm8, 3
                add         r9, rsi
           
           vperm2f128  ymm8, ymm8, ymm8, 1
           
                vextractps  dword ptr [r9], xmm8, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm8, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm8, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm8, 3
                add         r9, rsi
           
       
           
                vextractps  dword ptr [r9], xmm9, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm9, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm9, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm9, 3
                add         r9, rsi
           
           vperm2f128  ymm9, ymm9, ymm9, 1
           
                vextractps  dword ptr [r9], xmm9, 0
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm9, 1
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm9, 2
                add         r9, rsi
           
                vextractps  dword ptr [r9], xmm9, 3
                add         r9, rsi
           
       
    
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


