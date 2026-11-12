use std::{
    ffi::c_void,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{LocalFree, HANDLE},
    Security::{
        Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW},
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

fn user_descriptor() -> io::Result<LocalAllocation> {
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(token);
        let mut size = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, ptr::null_mut(), 0, &mut size);
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        // Aligned storage for TOKEN_USER and its trailing SID.
        let mut data = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(token.as_raw_handle(), TokenUser, data.as_mut_ptr().cast(), size, &mut size) == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*data.as_ptr().cast::<TOKEN_USER>();
        let mut sid = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _sid_allocation = LocalAllocation(sid.cast());
        let length = (0..).find(|&offset| *sid.add(offset) == 0).expect("Windows returns a terminated SID");
        let sid = std::slice::from_raw_parts(sid, length);
        let sddl: Vec<u16> =
            "D:P(A;;GA;;;".encode_utf16().chain(sid.iter().copied()).chain(")\0".encode_utf16()).collect();
        let mut descriptor = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, ptr::null_mut()) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(LocalAllocation(descriptor))
    }
}

pub fn listen(name: &str) -> io::Result<NamedPipeServer> {
    listen_instance(name, true, 1)
}

pub fn listen_instance(name: &str, first: bool, instances: usize) -> io::Result<NamedPipeServer> {
    let descriptor = user_descriptor()?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // Windows copies the descriptor when creating the pipe.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(instances)
            .create_with_security_attributes_raw(name, (&mut attributes as *mut SECURITY_ATTRIBUTES).cast())
    }
}

pub async fn accept(pipe: &NamedPipeServer, expected_pid: u32) -> io::Result<()> {
    loop {
        pipe.connect().await?;
        let mut pid = 0;
        if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } != 0 && pid == expected_pid {
            return Ok(());
        }
        pipe.disconnect()?;
    }
}

pub async fn connect(name: &str, expected_pid: u32) -> io::Result<NamedPipeClient> {
    loop {
        // SECURITY_SQOS_PRESENT with SECURITY_ANONYMOUS.
        match ClientOptions::new().security_qos_flags(0x0010_0000).open(name) {
            Ok(pipe) => {
                let mut pid = 0;
                if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } != 0 && pid == expected_pid {
                    return Ok(pipe);
                }
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn pipe_rejects_unexpected_peer_pids() {
        let name = format!(r"\\.\pipe\aiwin11-pid-test-{}", uuid::Uuid::new_v4());
        let server = listen(&name).unwrap();
        let client_error = connect(&name, u32::MAX).await.unwrap_err();
        assert_eq!(client_error.kind(), io::ErrorKind::PermissionDenied);

        drop(server);
        let server = listen(&name).unwrap();
        let client = connect(&name, std::process::id()).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(100), accept(&server, u32::MAX)).await.is_err());
        drop(client);
        let client = connect(&name, std::process::id()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), accept(&server, std::process::id())).await.unwrap().unwrap();
        drop(client);
    }
}
