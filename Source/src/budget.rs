use std::{
    io,
    mem::{size_of, size_of_val, zeroed},
    ptr::null,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::*,
        Memory::*,
        ProcessStatus::*,
        SystemInformation::{GetSystemInfo, SYSTEM_INFO},
        Threading::GetCurrentProcess,
    },
};

/// Decimal MB. Round down to a page so neither configured maximum exceeds this.
pub const MAX_BYTES: usize = 140_000_000;
pub struct Budget(HANDLE);

impl Drop for Budget {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl Budget {
    pub fn install() -> io::Result<Self> {
        unsafe {
            let mut system: SYSTEM_INFO = zeroed();
            GetSystemInfo(&mut system);
            let page = system.dwPageSize as usize;
            let ceiling = MAX_BYTES / page * page;
            let job = CreateJobObjectW(null(), null());
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let budget = Self(job);
            let process = GetCurrentProcess();
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            info.ProcessMemoryLimit = ceiling;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(job, process) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let (mut minimum, mut maximum, mut flags) = (0, 0, 0);
            if GetProcessWorkingSetSizeEx(process, &mut minimum, &mut maximum, &mut flags) == 0 {
                return Err(io::Error::last_os_error());
            }
            // Preserve a stricter inherited hard maximum when one already exists.
            let target = if flags & QUOTA_LIMITS_HARDWS_MAX_ENABLE != 0 {
                maximum.min(ceiling)
            } else {
                ceiling
            };
            if SetProcessWorkingSetSizeEx(
                process,
                minimum.min(target),
                target,
                QUOTA_LIMITS_HARDWS_MAX_ENABLE | QUOTA_LIMITS_HARDWS_MIN_DISABLE,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            if GetProcessWorkingSetSizeEx(process, &mut minimum, &mut maximum, &mut flags) == 0 {
                return Err(io::Error::last_os_error());
            }
            if maximum > ceiling || flags & QUOTA_LIMITS_HARDWS_MAX_ENABLE == 0 {
                return Err(io::Error::other(
                    "Windows did not confirm the working-set limit",
                ));
            }
            Ok(budget)
        }
    }
}

pub fn usage() -> Option<PROCESS_MEMORY_COUNTERS_EX> {
    unsafe {
        let mut info: PROCESS_MEMORY_COUNTERS_EX = zeroed();
        info.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let cb = info.cb;
        (GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut info as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            cb,
        ) != 0)
            .then_some(info)
    }
}

#[cfg(test)]
pub fn verify_allocation_limit(budget: &Budget) {
    unsafe {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        assert_ne!(
            QueryInformationJobObject(
                budget.0,
                JobObjectExtendedLimitInformation,
                (&mut info as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&info) as u32,
                std::ptr::null_mut()
            ),
            0
        );
        assert_ne!(
            info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            0
        );
        assert!(
            info.ProcessMemoryLimit > MAX_BYTES - 65536 && info.ProcessMemoryLimit <= MAX_BYTES
        );
        assert!(
            VirtualAlloc(null(), MAX_BYTES, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE).is_null(),
            "Windows accepted a single allocation larger than the remaining budget"
        );
    }
}
