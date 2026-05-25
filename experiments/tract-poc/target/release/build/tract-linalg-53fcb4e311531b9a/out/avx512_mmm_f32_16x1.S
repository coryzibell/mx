




.intel_syntax noprefix
.text
.p2align 5
.globl avx512_mmm_f32_16x1_0_22_1
avx512_mmm_f32_16x1_0_22_1:
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

	cmp rbx, 8
	jl .Lmain_loop_packed_packed_tail

.align 16
.Lmain_loop_packed_packed:
		// slow
	vbroadcastss xmm16, dword ptr [rcx]
	vbroadcastss xmm17, dword ptr [rcx + 4]
	vbroadcastss xmm18, dword ptr [rcx + 8]
	vbroadcastss xmm19, dword ptr [rcx + 12]

	// fast
	vmovups	   		xmm31, [rcx]
	vbroadcastss 	zmm16, xmm31
	valignd 		xmm17, xmm31, xmm31, 1
	vbroadcastss 	zmm17, xmm17
	valignd 		xmm18, xmm31, xmm31, 2
	vbroadcastss 	zmm18, xmm18
	valignd 		xmm19, xmm31, xmm31, 3
	vbroadcastss 	zmm19, xmm19

	// commmon
	vfmadd231ps		zmm0, zmm16, [rax + 0]
	vfmadd231ps		zmm1, zmm17, [rax + 64]
	vfmadd231ps		zmm2, zmm18, [rax + 128]
	vfmadd231ps		zmm3, zmm19, [rax + 192]

	add rcx, 16
	add rax, 256


    sub             rbx, 4
	cmp rbx,        4
	jge              .Lmain_loop_packed_packed

	
	   vaddps zmm0, zmm0, zmm1
	
	   vaddps zmm0, zmm0, zmm2
	
	   vaddps zmm0, zmm0, zmm3
	

    test    rbx, rbx
    jz      .Lnon_linear_loop

.align 16
.Lmain_loop_packed_packed_tail:
		vbroadcastss    zmm15, dword ptr [rcx]

    vmovups         zmm8, [rax]
    vfmadd231ps     zmm0, zmm15, zmm8

    add rcx, 4
	add rax, 64


	sub             rbx, 1
    jnz				.Lmain_loop_packed_packed_tail

    jmp      .Lnon_linear_loop

// vim: set syntax=asm :

// vim: set syntax=asm :

.Lscalar_min:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vminps          zmm0, zmm12, zmm0
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_max:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vmaxps          zmm0, zmm12, zmm0
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_add:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vaddps          zmm0, zmm12, zmm0
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_mul:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vmulps          zmm0, zmm12, zmm0
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vsubps          zmm0, zmm12, zmm0
        
    

    jmp    .Lnon_linear_loop

// vim: set syntax=asm :

.Lscalar_sub_flipped:
    vbroadcastss    zmm12, dword ptr [rdi + 8]
    
        
            vsubps          zmm0, zmm0, zmm12
        
    

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





    vmovups         zmm1,  [rax + 0]



    
        vminps zmm0, zmm1, zmm0
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_max:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm1,  [rax + 0]



    
        vmaxps zmm0, zmm1, zmm0
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_add:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm1,  [rax + 0]



    
        vaddps zmm0, zmm1, zmm0
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_mul:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm1,  [rax + 0]



    
        vmulps zmm0, zmm1, zmm0
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm1,  [rax + 0]



    
        vsubps zmm0, zmm1, zmm0
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_row_sub_flipped:
    mov             rax, [ rdi + 8 ]





    vmovups         zmm1,  [rax + 0]



    
        vsubps zmm0, zmm0, zmm1
    


    jmp .Lnon_linear_loop


// vim: set syntax=asm :

// vim: set syntax=asm :

.Lper_col_min:
    mov             rax, [ rdi + 8 ]








// 1 cols:1


    vbroadcastss    zmm1, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vminps zmm0, zmm1, zmm0
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_max:
    mov             rax, [ rdi + 8 ]








// 1 cols:1


    vbroadcastss    zmm1, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmaxps zmm0, zmm1, zmm0
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_add:
    mov             rax, [ rdi + 8 ]








// 1 cols:1


    vbroadcastss    zmm1, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vaddps zmm0, zmm1, zmm0
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_mul:
    mov             rax, [ rdi + 8 ]








// 1 cols:1


    vbroadcastss    zmm1, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vmulps zmm0, zmm1, zmm0
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub:
    mov             rax, [ rdi + 8 ]








// 1 cols:1


    vbroadcastss    zmm1, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm0, zmm1, zmm0
        
    


    jmp .Lnon_linear_loop

// vim: set syntax=asm :

.Lper_col_sub_flipped:
    mov             rax, [ rdi + 8 ]








// 1 cols:1


    vbroadcastss    zmm1, dword ptr [ rax ]
    add             rax, 4

    
        
        
            vsubps zmm0, zmm0, zmm1
        
    


    jmp .Lnon_linear_loop


// vim: set syntax=asm :

.Lload_tile:
    mov          r8, [rdi + 8]
    
        vmovups         zmm0, zmmword ptr [r8 + 0]
    

    jmp    .Lnon_linear_loop


.Ladd_unicast:
    mov     r10,    [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride

	cmp rsi, 4
	jne .Ladd_unicast_generic

	vaddps zmm0, zmm0, [r10]

    jmp    .Lnon_linear_loop

.Ladd_unicast_generic:
    mov r8, [0]
//     mov     eax,    0
// 
//     pinsrd  xmm14, eax, 0
//     add     eax,    esi
// 
//     pinsrd  xmm14, eax, 1
//     add     eax,    esi
// 
//     pinsrd  xmm14, eax, 2
//     add     eax,    esi
// 
//     pinsrd  xmm14, eax, 3
//     add     eax,    esi
// 
// 
//     pinsrd  xmm15, eax, 0
//     add     eax,    esi
// 
//     pinsrd  xmm15, eax, 1
//     add     eax,    esi
// 
//     pinsrd  xmm15, eax, 2
//     add     eax,    esi
// 
//     pinsrd  xmm15, eax, 3
//     add     eax,    esi
// 
//
//     vperm2f128      zmm14,  zmm14, zmm15,         32 // zmm14 <- xmm14::xmm15
//
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm0,   zmm0,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm1,   zmm1,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm2,   zmm2,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm3,   zmm3,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm4,   zmm4,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm5,   zmm5,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm6,   zmm6,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//     vpcmpeqd        zmm15,  zmm15, zmm15
//     vgatherdps      zmm12,  [ r10 + zmm14 ], zmm15
//
//     vaddps          zmm7,   zmm7,   zmm12
//     lea             r10, [ r10 + rsi * 8 ]
// 
//
    jmp    .Lnon_linear_loop

.Ladd_row_col_products:
    mov             rax, [ rdi + 8 ]
    mov             rbx, [ rdi + 16 ]

    vbroadcastss    zmm14, dword ptr [rbx]


    vmovups         zmm12,  [rax + 0]
    vfmadd231ps     zmm0, zmm12, zmm14

    jmp    .Lnon_linear_loop

.Lstore:
    mov     r8,     [rdi + 8]           // c ptr
    mov     rsi,    [rdi + 16]          // row stride

    cmp     rsi, 4
    jne      .Lstore_noncontiguous

	test r8, 63
	jnz .Lstore_unaligned

    vmovaps [r8], zmm0
    jmp     .Lnon_linear_loop

.Lstore_unaligned:
	vmovups [r8], zmm0
    jmp     .Lnon_linear_loop

.Lstore_noncontiguous:
    
        vextractf32x4 xmm8, zmm0, 0
        
            vextractps  dword ptr [r8], xmm8, 0
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 1
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 2
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 3
            add         r8, rsi
        
    
        vextractf32x4 xmm8, zmm0, 1
        
            vextractps  dword ptr [r8], xmm8, 0
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 1
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 2
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 3
            add         r8, rsi
        
    
        vextractf32x4 xmm8, zmm0, 2
        
            vextractps  dword ptr [r8], xmm8, 0
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 1
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 2
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 3
            add         r8, rsi
        
    
        vextractf32x4 xmm8, zmm0, 3
        
            vextractps  dword ptr [r8], xmm8, 0
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 1
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 2
            add         r8, rsi
        
            vextractps  dword ptr [r8], xmm8, 3
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


