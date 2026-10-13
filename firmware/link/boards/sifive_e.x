/* The emulation build's layout for QEMU's `sifive_e` machine (feature
   `board-sifive-e`), which runs the riscv32imc-unknown-none-elf images on
   its E31 core, an RV32IMAC: the images use none of the A extension's
   instructions. Code runs in place from the memory-mapped SPI flash at
   0x20000000, and data lives in 16 KiB of RAM at 0x80000000, as
   `info mtree` shows them. The mask ROM's reset code jumps to 0x20400000,
   where a HiFive1 (rev A) keeps the program, past its bootloader, so the
   image starts there: riscv-rt places its entry first in REGION_TEXT. The
   images use 512 KiB of it. riscv-rt's link.x includes this file. */
MEMORY
{
  FLASH : ORIGIN = 0x20400000, LENGTH = 512K
  RAM   : ORIGIN = 0x80000000, LENGTH = 16K
}

/* riscv-rt places its sections through these aliases. */
REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);

/* Unwind tables, which the images never read: see memory.x. */
SECTIONS
{
  /DISCARD/ : { *(.eh_frame) *(.eh_frame_hdr) }
}
