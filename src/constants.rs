pub const DEFAULT_MEMORY_BYTES_CAP: u32 = 1024;
// 1 slot = 2 bytes
pub const DEFAULT_CALLSTACK_SLOTS_CAP: usize = 256;
pub const DEFAULT_JS_CLOCK_ENABLED: bool = true;
pub const DEFAULT_JS_COPROCESSOR_ENABLED: bool = false;
pub const DEFAULT_JS_CLOCK_DEBUGGER_ENABLED: bool = false;
pub const MAX_ADDRESSABLE_MEMORY_BYTES: u32 = (u16::MAX as u32) + 1;
pub const DEFAULT_MAX_PHYS_REGS: u16 = 256;

pub const SCHEDULE_MAX_OPS_PER_CYCLE: usize = 64;
pub const SCHEDULE_MAX_COMPLEXITY_PER_CYCLE: usize = 64;
pub const SCHEDULE_MAX_STORE_MEM_PER_CYCLE: usize = 16;

pub fn validate_memory_bytes_cap(memory_bytes_cap: u32) -> anyhow::Result<u32> {
    anyhow::ensure!(
        memory_bytes_cap <= MAX_ADDRESSABLE_MEMORY_BYTES,
        "memory bytes cap {} exceeds 16-bit address space limit {}",
        memory_bytes_cap,
        MAX_ADDRESSABLE_MEMORY_BYTES
    );
    Ok(memory_bytes_cap)
}
