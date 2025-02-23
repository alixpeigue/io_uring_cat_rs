use core::str;
use std::{
    alloc::{alloc, dealloc, Layout},
    env,
    ffi::c_void,
    fs::{self},
    os::fd::AsRawFd,
    ptr, slice,
};

use anyhow::{anyhow, Context, Result};
use io_uring::{opcode, types, IoUring};
use libc::iovec;

const IOV_MAX: usize = 1024;

#[repr(C)]
struct FileInfo {
    pub file_sz: usize,
    pub iovecs: [iovec; 0],
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();

    if args.len() < 2 {
        return Err(anyhow!("Usage: {} <filename1> [<filename2> ...]", args[0]));
    }
    let mut ring = IoUring::new(args.len() as u32 - 1).context("Unable to setup uring")?;

    for arg in &args[1..args.len()] {
        submit_to_sq(arg, &mut ring).with_context(|| format!("Error reading from file {arg}"))?;
    }
    for _ in 1..args.len() {
        // At the moment, nothing guarantees that the files are printed in the order of the args,
        // but no tested input configuration resulted in printing the files out of order
        read_from_cq(&mut ring)?;
    }

    Ok(())
}

fn read_from_cq(ring: &mut IoUring) -> Result<()> {
    // ring.submit_and_wait(1)?;

    let cqe = ring.completion().next().context("Cannot get cqe")?;

    let fi: *mut FileInfo = cqe.user_data() as _;
    let fi_ref = unsafe { &mut *fi }; // Borrow instead of move

    let mut block_sz: usize = 4096;
    if fi_ref.file_sz / block_sz >= IOV_MAX {
        block_sz = (fi_ref.file_sz / IOV_MAX).next_power_of_two();
    }

    let mut blocks = fi_ref.file_sz / block_sz;
    if fi_ref.file_sz % block_sz != 0 {
        blocks += 1;
    }

    let iovecs = fi_ref.iovecs.as_mut_ptr();

    let buf_layout = Layout::from_size_align(block_sz, block_sz)?;
    for i in 0..blocks {
        let len = (unsafe { *iovecs.add(i as _) }).iov_len;
        let ptr = (unsafe { *iovecs.add(i as _) }).iov_base; // Simulate raw buffer

        unsafe {
            let u8_ptr = ptr as *mut u8;

            let byte_slice: &[u8] = slice::from_raw_parts(u8_ptr, len);

            let string = str::from_utf8_unchecked(byte_slice);
            print!("{}", string);

            dealloc(u8_ptr, buf_layout);
        }
    }
    let total_size = size_of::<FileInfo>() + blocks * size_of::<iovec>();
    let fi_layout = std::alloc::Layout::from_size_align(total_size, align_of::<FileInfo>())?;
    unsafe { dealloc(fi as _, fi_layout) };

    return Ok(());
}

fn submit_to_sq(arg: &str, ring: &mut IoUring) -> Result<()> {
    let file = fs::File::open(arg)?;
    let file_sz = file.metadata()?.len() as usize;
    let mut bytes_remaining = file_sz;

    let mut block_sz: usize = 1024;
    if file_sz / block_sz >= 1024 {
        block_sz = (file_sz / 1024).next_power_of_two();
    }

    let mut blocks = file_sz / block_sz;
    if file_sz % block_sz != 0 {
        blocks += 1;
    }

    let total_size = size_of::<FileInfo>() + blocks * size_of::<iovec>();
    let fi_layout = std::alloc::Layout::from_size_align(total_size, align_of::<FileInfo>())?;
    let fi: *mut FileInfo = unsafe { alloc(fi_layout) as _ };
    unsafe {
        ptr::write(
            fi,
            FileInfo {
                file_sz: file_sz as _,
                iovecs: [],
            },
        );
    }
    let iovecs = unsafe { (fi as *mut u8).add(size_of::<FileInfo>()) } as *mut iovec;
    let buf_layout = Layout::from_size_align(block_sz, block_sz)?;
    for i in 0..blocks {
        let len = if bytes_remaining < block_sz {
            bytes_remaining
        } else {
            block_sz
        };
        let buf: *mut c_void = unsafe { alloc(buf_layout) as _ };
        unsafe {
            *iovecs.add(i) = iovec {
                iov_base: buf,
                iov_len: len,
            };
        }
        if bytes_remaining >= block_sz {
            bytes_remaining -= block_sz;
        }
    }
    let read_e = opcode::Readv::new(types::Fd(file.as_raw_fd()), iovecs, blocks as _)
        .build()
        .user_data(fi as _);
    unsafe {
        ring.submission()
            .push(&read_e)
            .context("submission queue is full")?;
    }

    ring.submit()?;

    Ok(())
}
