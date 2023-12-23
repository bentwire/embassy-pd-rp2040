MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    /* Reserve 12MB for the application */
    FLASH : ORIGIN = 0x10000100, LENGTH = 12288K - LENGTH(BOOT2)
    /* Reserve 4MB for the storage areas */
    STORAGE_A: ORIGIN = ORIGIN(FLASH) + LENGTH(FLASH), LENGTH = 2048K
    STORAGE_B: ORIGIN = ORIGIN(STORAGE_A) + LENGTH(STORAGE_A), LENGTH = 2048K

    /* Pick one of the two options for RAM layout     */

    /* OPTION A: Use all RAM banks as one big block   */
    /* Reasonable, unless you are doing something     */
    /* really particular with DMA or other concurrent */
    /* access that would benefit from striping        */
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K

    /* OPTION B: Keep the unstriped sections separate */
    /* RAM: ORIGIN = 0x20000000, LENGTH = 256K        */
    /* SCRATCH_A: ORIGIN = 0x20040000, LENGTH = 4K    */
    /* SCRATCH_B: ORIGIN = 0x20041000, LENGTH = 4K    */
}

/* Offsets in to flash for the 2 storage areas. */
__storage_a_start = ORIGIN(STORAGE_A) - ORIGIN(BOOT2);
__storage_b_start = ORIGIN(STORAGE_B) - ORIGIN(BOOT2);
