/* The emulation build's layout for QEMU's `netduinoplus2` machine (feature
   `board-netduinoplus2`), which runs the thumbv7em-none-eabihf images: an
   STM32F405, a Cortex-M4F, with 1 MiB of flash at 0x08000000 (which QEMU
   also maps at 0, where the core reads its vector table at reset) and
   128 KiB of SRAM at 0x20000000, as `info mtree` shows them. Its 64 KiB of
   core-coupled RAM at 0x10000000 goes unused. The link test's generic layout
   uses the same two addresses. cortex-m-rt puts the stack at the top of
   RAM, and turns the FPU on before main on an eabihf target. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
