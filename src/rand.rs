//
//  flock - baa (with twenty instances of the letter "a")
//  Copyright (C) superwhiskers <whiskerdev@protonmail.com> 2022
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Affero General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Affero General Public License for more details.
//
//  You should have received a copy of the GNU Affero General Public License
//  along with this program.  If not, see <https://www.gnu.org/licenses/>.
//

use pcg_rand::Pcg64;
use rand::{RngCore, SeedableRng};
use std::{cell::UnsafeCell, rc::Rc};

thread_local! {
    // i'm just going to copy what rand's threadrng does for now
    static PCG_RAND_KEY: Rc<UnsafeCell<Pcg64>> = {
        Rc::new(UnsafeCell::new(Pcg64::from_entropy()))
    }
}

#[derive(Clone, Debug)]
pub struct PcgThreadRng {
    // the rationale for unsafecell here is the same as threadrng from rand. this should be
    // fine
    rng: Rc<UnsafeCell<Pcg64>>,
}

pub fn pcg_thread_rng() -> PcgThreadRng {
    let rng = PCG_RAND_KEY.with(|t| t.clone());
    PcgThreadRng { rng }
}

impl Default for PcgThreadRng {
    fn default() -> Self {
        pcg_thread_rng()
    }
}

impl RngCore for PcgThreadRng {
    #[inline(always)]
    fn next_u32(&mut self) -> u32 {
        // SAFETY: We must make sure to stop using `rng` before anyone else
        // creates another mutable reference
        let rng = unsafe { &mut *self.rng.get() };
        rng.next_u32()
    }

    #[inline(always)]
    fn next_u64(&mut self) -> u64 {
        // SAFETY: We must make sure to stop using `rng` before anyone else
        // creates another mutable reference
        let rng = unsafe { &mut *self.rng.get() };
        rng.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        // SAFETY: We must make sure to stop using `rng` before anyone else
        // creates another mutable reference
        let rng = unsafe { &mut *self.rng.get() };
        rng.fill_bytes(dest)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        // SAFETY: We must make sure to stop using `rng` before anyone else
        // creates another mutable reference
        let rng = unsafe { &mut *self.rng.get() };
        rng.try_fill_bytes(dest)
    }
}
