pub(crate) fn add_u16_to_buf(val: &u16, buf: &mut [u8], offset: &usize) -> anyhow::Result<usize> {
    let bytes = val.to_le_bytes();
    let n = bytes.len();
    buf[*offset..(n + (*offset))].copy_from_slice(&bytes);
    Ok(n)
}

pub(crate) fn add_f32_to_buf(val: &f32, buf: &mut [u8], offset: &usize) -> anyhow::Result<usize> {
    let bytes = val.to_le_bytes();
    let n = bytes.len();
    buf[*offset..(n + (*offset))].copy_from_slice(&bytes);
    Ok(n)
}

pub(crate) fn add_u64_to_buf(val: &u64, buf: &mut [u8], offset: &usize) -> anyhow::Result<usize> {
    let bytes = val.to_le_bytes();
    let n = bytes.len();
    buf[*offset..(n + (*offset))].copy_from_slice(&bytes);
    Ok(n)
}
