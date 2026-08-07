use std::ffi::c_void;

pub const SEED_COUNT: usize = 8;

#[derive(Clone)]
pub enum DuelSeed {
    None,
    Single(u32),
    Complicated([u32; SEED_COUNT]),
}

unsafe extern "C" {
    fn mtrandom_create(seeds: *const u32, len: usize) -> *mut c_void;
    fn mtrandom_create_value(value: u32) -> *mut c_void;
    fn mtrandom_destroy(handle: *mut c_void);
    fn mtrandom_rand(handle: *mut c_void) -> u32;
    fn mtrandom_discard(handle: *mut c_void, z: u64);
    fn mtrandom_get_random_integer(handle: *mut c_void, l: i32, h: i32) -> i32;
    fn mtrandom_shuffle_vector(handle: *mut c_void, data: *mut u32, count: usize);
}

unsafe impl Send for MTRandom {}

pub struct MTRandom {
    handle: *mut c_void,
    seed_sequence: [u32; SEED_COUNT],
}

impl MTRandom {
    pub fn new(seed: DuelSeed) -> Self {
        let seed_array = match seed {
            DuelSeed::None => {
                let mut seeds = [0u32; SEED_COUNT];
                for i in 0..SEED_COUNT {
                    seeds[i] = rand::random();
                }
                seeds
            }
            DuelSeed::Single(s) => [s; SEED_COUNT],
            DuelSeed::Complicated(seq) => seq,
        };
        let handle = unsafe { mtrandom_create(seed_array.as_ptr(), seed_array.len()) };
        Self { handle, seed_sequence: seed_array }
    }

    pub fn rand(&self) -> u32 {
        unsafe { mtrandom_rand(self.handle) }
    }

    pub fn discard(&self, z: u64) {
        unsafe { mtrandom_discard(self.handle, z) };
    }

    pub fn get_random_integer(&self, l: i32, h: i32) -> i32 {
        unsafe { mtrandom_get_random_integer(self.handle, l, h) }
    }

    pub fn shuffle_deck(&self, deck: &mut [u32]) {
        unsafe { mtrandom_shuffle_vector(self.handle, deck.as_mut_ptr(), deck.len()) };
    }

    pub fn seed_sequence(&self) -> &[u32; SEED_COUNT] {
        &self.seed_sequence
    }
}

impl Drop for MTRandom {
    fn drop(&mut self) {
        unsafe { mtrandom_destroy(self.handle) };
    }
}
