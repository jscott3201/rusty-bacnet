/* A generic small MCU: 256 KiB of flash and 64 KiB of RAM, at the addresses
   most Cortex-M vendors use. The images only have to link, so no particular
   chip is meant, and the RISC-V images use the same layout. The runtimes'
   link.x scripts include this file before their own SECTIONS. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}

/* riscv-rt places its sections through these aliases; cortex-m-rt uses
   FLASH and RAM directly and ignores them. */
REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);

/* Unwind tables. riscv-rt's link.x doesn't place .eh_frame, so the linker
   would put it in flash: 2.6 KB for the MS/TP image. Only unwinding reads
   it, and these images never unwind: they build with panic = "abort", the
   `firmware` profile's setting and these targets' default. If that ever
   changed, a panic would still abort, as it does now. cortex-m-rt already
   drops ARM's unwind tables (.ARM.exidx) the same way. */
SECTIONS
{
  /DISCARD/ : { *(.eh_frame) *(.eh_frame_hdr) }
}
