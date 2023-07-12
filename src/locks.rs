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

use parking_lot::RwLock;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
};
use tracing::{debug, trace};

//TODO(superwhiskers): provide a means to set a handler if a thread panics to clean up
//                     resources/revert changes
//TODO(superwhiskers): provide a way to garbage collect unlocked entries within the map

/// A structure providing a means to associate strings with locks on some external data
///
/// If the data is locked, a lock operation returns immediately
pub struct LockMap(RwLock<HashMap<String, AtomicBool>>);

impl LockMap {
    pub fn new() -> &'static Self {
        Box::leak(Box::new(Self(RwLock::new(HashMap::new()))))
    }

    pub fn with_capacity(capacity: usize) -> &'static Self {
        Box::leak(Box::new(Self(RwLock::new(HashMap::with_capacity(
            capacity,
        )))))
    }

    pub fn lock(&'static self, key: &str) -> Option<LockMapGuard> {
        trace!("requested to lock for key \"{}\"", key);

        let inner = self.0.read();

        if let Some(lock) = inner.get(key) {
            debug!("requesting existing lock for key \"{}\"", key);

            if lock
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                debug!("failed to lock for key \"{}\"", key);

                None
            } else {
                debug!("successfully locked for key \"{}\"", key);

                Some(LockMapGuard(self, key.to_string()))
            }
        } else {
            trace!("creating new lock for key \"{}\"", key);

            drop(inner); // we don't want to deadlock
            self.new_lock(key)
        }
    }

    fn new_lock(&'static self, key: &str) -> Option<LockMapGuard> {
        trace!("requested to create a new lock for key \"{}\"", key);

        let mut inner = self.0.write();

        debug!("creating new lock for key \"{}\"", key);

        if inner
            .try_insert(key.to_string(), AtomicBool::new(true))
            .is_ok()
        {
            Some(LockMapGuard(self, key.to_string()))
        } else {
            None
        }
    }
}

/// A guard over data locked by the [`LockMap`]
pub struct LockMapGuard(&'static LockMap, String);

impl Drop for LockMapGuard {
    fn drop(&mut self) {
        trace!("requested to drop lock for key \"{}\"", self.1.as_str());

        let inner = self.0 .0.read();

        debug!("dropping lock for key \"{}\"", self.1.as_str());

        //SAFETY: there's no way this isn't in the map as we never remove anything
        unsafe { inner.get(self.1.as_str()).unwrap_unchecked() }.store(false, Ordering::Release);
    }
}
