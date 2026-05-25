




.intel_syntax noprefix
.text
.p2align 5
.globl avx512_mmm_f32_16x8_0_22_1
avx512_mmm_f32_16x8_0_22_1:
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

	cmp rbx, 2
	jl .Lmain_loop_packed_packed_tail

.align 16
.Lmain_loop_packed_packed:
		// Tile size: 1x8
	// Accumulators: 0-7
	// Col regs: 8-14
	// Row regs: 15


    vmovaps         zmm15,  [rax]

    vbroadcastss    zmm8, dword ptr [rcx + 0 * 4]
    vfmadd231ps     zmm0, zmm15, zmm8

    vbroadcastss    zmm9, dword ptr [rcx + 1 * 4]
    vfmadd231ps     zmm1, zmm15, zmm9

    vbroadcastss    zmm10, dword ptr [rcx + 2 * 4]
    vfmadd231ps     zmm2, zmm15, zmm10

    vbroadcastss    zmm11, dword ptr [rcx + 3 * 4]
    vfmadd231ps     zmm3, zmm15, zmm11

    vbroadcastss    zmm12, dword ptr [rcx + 4 * 4]
    vfmadd231ps     zmm4, zmm15, zmm12

    vbroadcastss    zmm13, dword ptr [rcx + 5 * 4]
    vfmadd231ps     zmm5, zmm15, zmm13

    vbroadcastss    zmm10, dword ptr [rcx + 6 * 4]
    vfmadd231ps     zmm6, zmm15, zmm10

    vbroadcastss    zmm11, dword ptr [rcx + 7 * 4]
    vfmadd231ps     zmm7, zmm15, zmm11


    vmovaps         zmm15,  [rax+64]

    vbroadcastss    zmm8, dword ptr [rcx + 8 * 4]
    vfmadd231ps     zmm0, zmm15, zmm8

    vbroadcastss    zmm9, dword ptr [rcx + 9 * 4]
    vfmadd231ps     zmm1, zmm15, zmm9

    vbroadcastss    zmm10, dword ptr [rcx + 10 * 4]
    vfmadd231ps     zmm2, zmm15, zmm10

    vbroadcastss    zmm11, dword ptr [rcx + 11 * 4]
    vfmadd231ps     zmm3, zmm15, zmm11

    vbroadcastss    zmm12, dword ptr [rcx + 12 * 4]
    vfmadd231ps     zmm4, zmm15, zmm12

    vbroadcastss    zmm13, dword ptr [rcx + 13 * 4]
    vfmadd231ps     zmm5, zmm15, zmm13

    vbroadcastss    zmm10, dword ptr [rcx + 14 * 4]
    vfmadd231ps     zmm6, zmm15, zmm10

    vbroadcastss    zmm11, dword ptr [rcx + 15 * 4]
    vfmadd231ps     zmm7, zmm15, zmm11

	add rcx, 64
	add rax, 128

    sub             rbx, 2
	cmp rbx,        2
	jge              .Lmain_loop_packed_packed

    test    rbx, rbx
    jz      .Lnon_linear_loop

.align 16
.Lmain_loop_packed_packed_tail:
		// Tile size: 1x8
	// Accumulators: 0-7
	// Col regs: 8-14
	// Row regs: 15

    vmovaps         zmm15,  [rax]

    vbroadcastss    zmm8, dword ptr [rcx + 0 * 4]
    vfmadd231ps     zmm0, zmm15, zmm8

    vbroadcastss    zmm9, dword ptr [rcx + 1 * 4]
    vfmadd231ps     zmm1, zmm15, zmm9

    vbroadcastss    zmm10, dword ptr [rcx + 2 * 4]
    vfmadd231ps     zmm2, zmm15, zmm10

    vbroadcastss    zmm11, dword ptr [rcx + 3 * 4]
    vfmadd231ps     zmm3, zmm15, zmm11

    vbroadcastss    zmm12, dword ptr [rcx + 4 * 4]
    vfmadd231ps     zmm4, zmm15, zmm12

    vbroadcastss    zmm13, dword ptr [rcx + 5 * 4]
    vfmadd231ps     zmm5, zmm15, zmm13

    vbroadcastss    zmm10, dword ptr [rcx + 6 * 4]
    vfmadd231ps     zmm6, zmm15, zmm10

    vbroadcastss    zmm11, dword ptr [rcx + 7 * 4]
    vfmadd231ps     zmm7, zmm15, zmm11

	add rcx, 32
	add rax, 64


	sub             rbx, 1
    jnz				.Lmain_loop_packed_packed_tail

    jmp      .Lnon_linear_loop

// vim: set syntax=asm :

// vim: set syntax=asm :

.Lscalar_min:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vminps          zmm0, zmm12, zmm0
        
            vminps          zmm1, zmm12, zmm1
        
            vminps          zmm2, zmm12, zmm2
        
            vminps          zmm3, zmm12, zmm3
        
            vminps          zmm4, zmm12, zmm4
        
            vminps          zmm5, zmm12, zmm5
        
            vminps          zmm6, zmm12, zmm6
        
            vminps          zmm7, zmm12, zmm7
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_max:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vmaxps          zmm0, zmm12, zmm0
        
            vmaxps          zmm1, zmm12, zmm1
        
            vmaxps          zmm2, zmm12, zmm2
        
            vmaxps          zmm3, zmm12, zmm3
        
            vmaxps          zmm4, zmm12, zmm4
        
            vmaxps          zmm5, zmm12, zmm5
        
            vmaxps          zmm6, zmm12, zmm6
        
            vmaxps          zmm7, zmm12, zmm7
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_add:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vaddps          zmm0, zmm12, zmm0
        
            vaddps          zmm1, zmm12, zmm1
        
            vaddps          zmm2, zmm12, zmm2
        
            vaddps          zmm3, zmm12, zmm3
        
            vaddps          zmm4, zmm12, zmm4
        
            vaddps          zmm5, zmm12, zmm5
        
            vaddps          zmm6, zmm12, zmm6
        
            vaddps          zmm7, zmm12, zmm7
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_mul:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vmulps          zmm0, zmm12, zmm0
        
            vmulps          zmm1, zmm12, zmm1
        
            vmulps          zmm2, zmm12, zmm2
        
            vmulps          zmm3, zmm12, zmm3
        
            vmulps          zmm4, zmm12, zmm4
        
            vmulps          zmm5, zmm12, zmm5
        
            vmulps          zmm6, zmm12, zmm6
        
            vmulps          zmm7, zmm12, zmm7
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vsubps          zmm0, zmm12, zmm0
        
            vsubps          zmm1, zmm12, zmm1
        
            vsubps          zmm2, zmm12, zmm2
        
            vsubps          zmm3, zmm12, zmm3
        
            vsubps          zmm4, zmm12, zmm4
        
            vsubps          zmm5, zmm12, zmm5
        
            vsubps          zmm6, zmm12, zmm6
        
            vsubps          zmm7, zmm12, zmm7
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub_flipped:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vsubps          zmm0, zmm0, zmm12
        
            vsubps          zmm1, zmm1, zmm12
        
            vsubps          zmm2, zmm2, zmm12
        
            vsubps          zmm3, zmm3, zmm12
        
            vsubps          zmm4, zmm4, zmm12
        
            vsubps          zmm5, zmm5, zmm12
        
            vsubps          zmm6, zmm6, zmm12
        
            vsubps          zmm7, zmm7, zmm12
        
    

    jmp    .Lnon_linear_loop


.Lleaky_relu:
    // can only use zmm12 to zmm15
    // ymm15 <- alpha
    vbroadcastss    zmm15, dword ptr [rdi + 8]
    // ymm14 <- all zero
    vpxorq          zmm14, zmm14, zmm14

    
        vcmpps      k1, zmm0, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm0 {k1}, zmm0, zmm15
    
        vcmpps      k1, zmm1, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm1 {k1}, zmm1, zmm15
    
        vcmpps      k1, zmm2, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm2 {k1}, zmm2, zmm15
    
        vcmpps      k1, zmm3, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm3 {k1}, zmm3, zmm15
    
        vcmpps      k1, zmm4, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm4 {k1}, zmm4, zmm15
    
        vcmpps      k1, zmm5, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm5 {k1}, zmm5, zmm15
    
        vcmpps      k1, zmm6, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm6 {k1}, zmm6, zmm15
    
        vcmpps      k1, zmm7, zmm14, 1 // 1 means LT
        // ymm12 <- alpha * x if < 0
        vmulps      zmm7 {k1}, zmm7, zmm15
    
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





    vmovups         zmm8,  [rax + 0]



    
        vminps zmm0, zmm8, zmm0
    
        vminps zmm1, zmm8, zmm1
    
        vminps zmm2, zmm8, zmm2
    
        vminps zmm3, zmm8, zmm3
    
        vminps zmm4, zmm8, zmm4
    
        vminps zmm5, zmm8, zmm5
    
        vminps zmm6, zmm8, zmm6
    
        vminps zmm7, zmm8, zmm7
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_max:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm8,  [rax + 0]



    
        vmaxps zmm0, zmm8, zmm0
    
        vmaxps zmm1, zmm8, zmm1
    
        vmaxps zmm2, zmm8, zmm2
    
        vmaxps zmm3, zmm8, zmm3
    
        vmaxps zmm4, zmm8, zmm4
    
        vmaxps zmm5, zmm8, zmm5
    
        vmaxps zmm6, zmm8, zmm6
    
        vmaxps zmm7, zmm8, zmm7
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_add:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm8,  [rax + 0]



    
        vaddps zmm0, zmm8, zmm0
    
        vaddps zmm1, zmm8, zmm1
    
        vaddps zmm2, zmm8, zmm2
    
        vaddps zmm3, zmm8, zmm3
    
        vaddps zmm4, zmm8, zmm4
    
        vaddps zmm5, zmm8, zmm5
    
        vaddps zmm6, zmm8, zmm6
    
        vaddps zmm7, zmm8, zmm7
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_mul:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm8,  [rax + 0]



    
        vmulps zmm0, zmm8, zmm0
    
        vmulps zmm1, zmm8, zmm1
    
        vmulps zmm2, zmm8, zmm2
    
        vmulps zmm3, zmm8, zmm3
    
        vmulps zmm4, zmm8, zmm4
    
        vmulps zmm5, zmm8, zmm5
    
        vmulps zmm6, zmm8, zmm6
    
        vmulps zmm7, zmm8, zmm7
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm8,  [rax + 0]



    
        vsubps zmm0, zmm8, zmm0
    
        vsubps zmm1, zmm8, zmm1
    
        vsubps zmm2, zmm8, zmm2
    
        vsubps zmm3, zmm8, zmm3
    
        vsubps zmm4, zmm8, zmm4
    
        vsubps zmm5, zmm8, zmm5
    
        vsubps zmm6, zmm8, zmm6
    
        vsubps zmm7, zmm8, zmm7
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub_flipped:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm8,  [rax + 0]



    
        vsubps zmm0, zmm0, zmm8
    
        vsubps zmm1, zmm1, zmm8
    
        vsubps zmm2, zmm2, zmm8
    
        vsubps zmm3, zmm3, zmm8
    
        vsubps zmm4, zmm4, zmm8
    
        vsubps zmm5, zmm5, zmm8
    
        vsubps zmm6, zmm6, zmm8
    
        vsubps zmm7, zmm7, zmm8
    


    jmp .Lnon_linear_loop


// vim: set syntax=asm :

// vim: set syntax=asm :

.Lper_col_min:
    mov             rax, [ rdi + 8 ]








// 8 cols:8


    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm0, zmm8, zmm0
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm1, zmm8, zmm1
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm2, zmm8, zmm2
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm3, zmm8, zmm3
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm4, zmm8, zmm4
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm5, zmm8, zmm5
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm6, zmm8, zmm6
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm7, zmm8, zmm7
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_max:
    mov             rax, [ rdi + 8 ]








// 8 cols:8


    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm0, zmm8, zmm0
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm1, zmm8, zmm1
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm2, zmm8, zmm2
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm3, zmm8, zmm3
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm4, zmm8, zmm4
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm5, zmm8, zmm5
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm6, zmm8, zmm6
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm7, zmm8, zmm7
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_add:
    mov             rax, [ rdi + 8 ]








// 8 cols:8


    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm0, zmm8, zmm0
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm1, zmm8, zmm1
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm2, zmm8, zmm2
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm3, zmm8, zmm3
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm4, zmm8, zmm4
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm5, zmm8, zmm5
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm6, zmm8, zmm6
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm7, zmm8, zmm7
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_mul:
    mov             rax, [ rdi + 8 ]








// 8 cols:8


    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm0, zmm8, zmm0
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm1, zmm8, zmm1
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm2, zmm8, zmm2
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm3, zmm8, zmm3
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm4, zmm8, zmm4
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm5, zmm8, zmm5
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm6, zmm8, zmm6
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm7, zmm8, zmm7
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub:
    mov             rax, [ rdi + 8 ]








// 8 cols:8


    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm0, zmm8, zmm0
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm1, zmm8, zmm1
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm2, zmm8, zmm2
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm3, zmm8, zmm3
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm4, zmm8, zmm4
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm5, zmm8, zmm5
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm6, zmm8, zmm6
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm7, zmm8, zmm7
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub_flipped:
    mov             rax, [ rdi + 8 ]








// 8 cols:8


    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm0, zmm0, zmm8
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm1, zmm1, zmm8
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm2, zmm2, zmm8
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm3, zmm3, zmm8
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm4, zmm4, zmm8
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm5, zmm5, zmm8
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm6, zmm6, zmm8
        
    

    vbroadcastss    zmm8, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm7, zmm7, zmm8
        
    


    jmp .Lnon_linear_loop


// vim: set syntax=asm :

.Lload_tile:
    mov          r8, [rdi + 8]
    
        vmovups         zmm0, zmmword ptr [r8 + 0]
    
        vmovups         zmm1, zmmword ptr [r8 + 64]
    
        vmovups         zmm2, zmmword ptr [r8 + 128]
    
        vmovups         zmm3, zmmword ptr [r8 + 192]
    
        vmovups         zmm4, zmmword ptr [r8 + 256]
    
        vmovups         zmm5, zmmword ptr [r8 + 320]
    
        vmovups         zmm6, zmmword ptr [r8 + 384]
    
        vmovups         zmm7, zmmword ptr [r8 + 448]
    

    jmp    .Lnon_linear_loop


.Ladd_unicast:

    mov     r10,    [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride
    mov     rbx,    [rdi + 24]          // col stride

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


    pinsrd  xmm12, eax, 0
    add     eax,    esi

    pinsrd  xmm12, eax, 1
    add     eax,    esi

    pinsrd  xmm12, eax, 2
    add     eax,    esi

    pinsrd  xmm12, eax, 3
    add     eax,    esi


    pinsrd  xmm13, eax, 0
    add     eax,    esi

    pinsrd  xmm13, eax, 1
    add     eax,    esi

    pinsrd  xmm13, eax, 2
    add     eax,    esi

    pinsrd  xmm13, eax, 3
    add     eax,    esi


    vperm2f128      ymm14,  ymm14, ymm15,         32 // ymm14 <- xmm14::xmm15
    vperm2f128      ymm13,  ymm12, ymm13,         32 // ymm12 <- xmm12::xmm13
    vinsertf32x8    zmm14, zmm14, ymm13, 1


    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm0,   zmm0,   zmm12

    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm1,   zmm1,   zmm12

    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm2,   zmm2,   zmm12

    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm3,   zmm3,   zmm12

    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm4,   zmm4,   zmm12

    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm5,   zmm5,   zmm12

    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm6,   zmm6,   zmm12

    kxnorw k1,k1,k1
    vgatherdps      zmm12{k1},  [ r10 + zmm14 ]
    add     r10, rbx
    vaddps          zmm7,   zmm7,   zmm12


    jmp    .Lnon_linear_loop

.Ladd_row_col_products:
    mov             rax, [ rdi + 8 ]
    mov             rbx, [ rdi + 16 ]

    vmovups         zmm12, zmmword ptr [rax]


    vbroadcastss    zmm14, dword ptr [rbx + 0 ]
    vfmadd231ps     zmm0,   zmm12, zmm14

    vbroadcastss    zmm14, dword ptr [rbx + 4 ]
    vfmadd231ps     zmm1,   zmm12, zmm14

    vbroadcastss    zmm14, dword ptr [rbx + 8 ]
    vfmadd231ps     zmm2,   zmm12, zmm14

    vbroadcastss    zmm14, dword ptr [rbx + 12 ]
    vfmadd231ps     zmm3,   zmm12, zmm14

    vbroadcastss    zmm14, dword ptr [rbx + 16 ]
    vfmadd231ps     zmm4,   zmm12, zmm14

    vbroadcastss    zmm14, dword ptr [rbx + 20 ]
    vfmadd231ps     zmm5,   zmm12, zmm14

    vbroadcastss    zmm14, dword ptr [rbx + 24 ]
    vfmadd231ps     zmm6,   zmm12, zmm14

    vbroadcastss    zmm14, dword ptr [rbx + 28 ]
    vfmadd231ps     zmm7,   zmm12, zmm14

    jmp    .Lnon_linear_loop

.Lstore:
    mov     r8,     [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride
    mov     rbx,    [rdi + 24]          // col stride

    // tops of cols
    lea     r9,     [ r8 + rbx ]
    lea     r10,    [ r8 + 2 * rbx ]
    lea     r12,    [ r8 + 4 * rbx ]
    lea     r11,    [ r10 + rbx ]
    lea     r13,    [ r12 + rbx ]
    lea     r14,    [ r12 + 2 * rbx ]
    lea     r15,    [ r13 + 2 * rbx ]
    
    
        
            vextractf32x4 xmm8, zmm0, 0
        
            vextractf32x4 xmm9, zmm1, 0
        
            vextractf32x4 xmm10, zmm2, 0
        
            vextractf32x4 xmm11, zmm3, 0
        
            vextractf32x4 xmm12, zmm4, 0
        
            vextractf32x4 xmm13, zmm5, 0
        
            vextractf32x4 xmm14, zmm6, 0
        
            vextractf32x4 xmm15, zmm7, 0
        
        
            
                vextractps  dword ptr [r8], xmm8, 0
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 0
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 0
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 0
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 0
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 0
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 0
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 0
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 1
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 1
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 1
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 1
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 1
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 1
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 1
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 1
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 2
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 2
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 2
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 2
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 2
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 2
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 2
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 2
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 3
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 3
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 3
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 3
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 3
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 3
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 3
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 3
                add         r15, rsi
            
        
    
        
            vextractf32x4 xmm8, zmm0, 1
        
            vextractf32x4 xmm9, zmm1, 1
        
            vextractf32x4 xmm10, zmm2, 1
        
            vextractf32x4 xmm11, zmm3, 1
        
            vextractf32x4 xmm12, zmm4, 1
        
            vextractf32x4 xmm13, zmm5, 1
        
            vextractf32x4 xmm14, zmm6, 1
        
            vextractf32x4 xmm15, zmm7, 1
        
        
            
                vextractps  dword ptr [r8], xmm8, 0
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 0
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 0
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 0
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 0
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 0
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 0
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 0
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 1
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 1
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 1
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 1
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 1
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 1
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 1
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 1
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 2
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 2
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 2
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 2
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 2
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 2
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 2
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 2
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 3
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 3
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 3
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 3
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 3
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 3
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 3
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 3
                add         r15, rsi
            
        
    
        
            vextractf32x4 xmm8, zmm0, 2
        
            vextractf32x4 xmm9, zmm1, 2
        
            vextractf32x4 xmm10, zmm2, 2
        
            vextractf32x4 xmm11, zmm3, 2
        
            vextractf32x4 xmm12, zmm4, 2
        
            vextractf32x4 xmm13, zmm5, 2
        
            vextractf32x4 xmm14, zmm6, 2
        
            vextractf32x4 xmm15, zmm7, 2
        
        
            
                vextractps  dword ptr [r8], xmm8, 0
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 0
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 0
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 0
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 0
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 0
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 0
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 0
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 1
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 1
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 1
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 1
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 1
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 1
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 1
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 1
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 2
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 2
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 2
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 2
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 2
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 2
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 2
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 2
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 3
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 3
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 3
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 3
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 3
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 3
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 3
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 3
                add         r15, rsi
            
        
    
        
            vextractf32x4 xmm8, zmm0, 3
        
            vextractf32x4 xmm9, zmm1, 3
        
            vextractf32x4 xmm10, zmm2, 3
        
            vextractf32x4 xmm11, zmm3, 3
        
            vextractf32x4 xmm12, zmm4, 3
        
            vextractf32x4 xmm13, zmm5, 3
        
            vextractf32x4 xmm14, zmm6, 3
        
            vextractf32x4 xmm15, zmm7, 3
        
        
            
                vextractps  dword ptr [r8], xmm8, 0
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 0
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 0
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 0
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 0
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 0
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 0
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 0
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 1
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 1
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 1
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 1
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 1
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 1
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 1
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 1
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 2
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 2
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 2
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 2
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 2
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 2
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 2
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 2
                add         r15, rsi
            
        
            
                vextractps  dword ptr [r8], xmm8, 3
                add         r8, rsi
            
                vextractps  dword ptr [r9], xmm9, 3
                add         r9, rsi
            
                vextractps  dword ptr [r10], xmm10, 3
                add         r10, rsi
            
                vextractps  dword ptr [r11], xmm11, 3
                add         r11, rsi
            
                vextractps  dword ptr [r12], xmm12, 3
                add         r12, rsi
            
                vextractps  dword ptr [r13], xmm13, 3
                add         r13, rsi
            
                vextractps  dword ptr [r14], xmm14, 3
                add         r14, rsi
            
                vextractps  dword ptr [r15], xmm15, 3
                add         r15, rsi
            
        
    

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


