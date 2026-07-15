use std::sync::Arc;

pub(crate) struct BufferPool {
    pool: std::sync::Mutex<Vec<Vec<u8>>>,
    pub(crate) buf_size: usize,
}

impl BufferPool {
    pub fn new(capacity: usize, buf_size: usize) -> Self {
        let mut pool = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            pool.push(vec![0; buf_size]);
        }
        Self {
            pool: std::sync::Mutex::new(pool),
            buf_size,
        }
    }

    pub fn acquire(&self) -> Vec<u8> {
        if let Ok(mut pool) = self.pool.lock() {
            if let Some(buf) = pool.pop() {
                return buf;
            }
        }
        vec![0; self.buf_size]
    }

    pub fn release(&self, mut buf: Vec<u8>) {
        if buf.capacity() < self.buf_size {
            buf.resize(self.buf_size, 0);
        }
        if let Ok(mut pool) = self.pool.lock() {
            if pool.len() < pool.capacity() {
                pool.push(buf);
            }
        }
    }
}

pub(crate) struct PooledBuffer {
    data: Vec<u8>,
    pool: Arc<BufferPool>,
}

impl PooledBuffer {
    pub fn new(data: Vec<u8>, pool: Arc<BufferPool>) -> Self {
        Self { data, pool }
    }
}

impl std::ops::Deref for PooledBuffer {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl std::ops::DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl AsRef<[u8]> for PooledBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        let mut buf = std::mem::take(&mut self.data);
        buf.resize(self.pool.buf_size, 0);
        self.pool.release(buf);
    }
}
