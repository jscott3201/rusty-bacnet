/* riscv32imac-unknown-none-elf's layout (#2363): the objects and reporting
   images, the only ones built for it, sized for an ESP32-C6-class part, an
   RV32IMAC with 512 KiB of SRAM that runs in place from external SPI flash
   of 4 MiB or more. The images get 512 KiB of that flash, room for COV and
   intrinsic event reporting (#2303), and the generic 64 KiB of RAM.
   Addresses are memory.x's: the images only have to link, so no
   particular chip's memory map is meant. riscv-rt's link.x includes this
   file before its own SECTIONS. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}

/* riscv-rt places its sections through these aliases. */
REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);

/* The unwind tables, dropped as memory.x drops them: the images build
   with panic = "abort" and never unwind. */
SECTIONS
{
  /DISCARD/ : { *(.eh_frame) *(.eh_frame_hdr) }
}
