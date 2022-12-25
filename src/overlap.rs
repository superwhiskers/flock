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

//TODO(superwhiskers): find a place to put this

use std::{
    assert_matches::debug_assert_matches,
    cmp,
    fmt::Debug,
    hint::unreachable_unchecked,
    ops::{Bound, RangeBounds, Sub},
};

/// The overlap between two ranges
#[derive(Debug)]
pub enum Overlap<T> {
    // The two ranges overlap the amount of the provided value
    Positive(T),

    // The two ranges are separated by the provided value
    Negative(T),

    // The overlap is approaching positive infinity from the provided value
    PositiveInfinity(T),

    // The overlap is approaching negative infinity from the provided value
    NegativeInfinity(T),

    // The overlap covers the entire range of possible values
    Infinity,
}

/// Normalize the direction of the provided [`RangeBounds`] (increasing), restrict them to a
/// subset of [`Bound`]s that only consists of [`Bound::Included`] and [`Bound::Unbounded`],
/// and dereference the bounds
pub fn normalize_bounds<T>(b: impl RangeBounds<T>) -> (Bound<T>, Bound<T>)
where
    T: Ord + Copy,
{
    match (b.start_bound(), b.end_bound()) {
        (Bound::Included(&a), Bound::Included(&b))
        | (Bound::Excluded(&a), Bound::Included(&b))
        | (Bound::Included(&a), Bound::Excluded(&b))
        | (Bound::Excluded(&a), Bound::Excluded(&b)) => {
            let (aw, bw) = (Bound::Included(a), Bound::Included(b));
            if a < b {
                (aw, bw)
            } else {
                (bw, aw)
            }
        }
        (Bound::Unbounded, Bound::Included(&b)) | (Bound::Unbounded, Bound::Excluded(&b)) => {
            let bw = Bound::Included(b);
            (Bound::Unbounded, bw)
        }
        (Bound::Included(&a), Bound::Unbounded) | (Bound::Excluded(&a), Bound::Unbounded) => {
            let aw = Bound::Included(a);
            (aw, Bound::Unbounded)
        }
        (Bound::Unbounded, Bound::Unbounded) => (Bound::Unbounded, Bound::Unbounded),
    }
}

/// Pick the correct bound for overlap calculation between bounds `s1` and `s2`, where the
/// opposing bounds are `e1` and `e2`, respectively
pub fn choose_bound<T>(
    predicate: impl Fn(T, T) -> T,
    (s1, s2): (Bound<T>, Bound<T>),
    (e1, e2): (Bound<T>, Bound<T>),
) -> Bound<T> {
    match (s1, s2) {
        (Bound::Included(a), Bound::Included(b)) => Bound::Included(predicate(a, b)),
        (Bound::Unbounded, Bound::Included(b)) => Bound::Included(match e1 {
            Bound::Included(a) => predicate(a, b),
            Bound::Unbounded => b,
            Bound::Excluded(_) => unsafe { unreachable_unchecked() },
        }),
        (Bound::Included(a), Bound::Unbounded) => Bound::Included(match e2 {
            Bound::Included(b) => predicate(a, b),
            Bound::Unbounded => a,
            Bound::Excluded(_) => unsafe { unreachable_unchecked() },
        }),
        (Bound::Unbounded, Bound::Unbounded) => match (e1, e2) {
            (Bound::Included(a), Bound::Included(b)) => Bound::Included(predicate(a, b)),
            (Bound::Unbounded, Bound::Included(_))
            | (Bound::Included(_), Bound::Unbounded)
            | (Bound::Unbounded, Bound::Unbounded) => Bound::Unbounded,
            (Bound::Excluded(_), _) | (_, Bound::Excluded(_)) => unsafe { unreachable_unchecked() },
        },
        (Bound::Excluded(_), _) | (_, Bound::Excluded(_)) => unsafe { unreachable_unchecked() },
    }
}

/// Calculate the overlap between two ranges, including negative overlap and handling
/// infinite ranges
pub fn overlap<T>(a: impl RangeBounds<T>, b: impl RangeBounds<T>) -> Overlap<T>
where
    T: Sub<Output = T> + Ord + Debug + Copy,
{
    // these may not necessarily be in the order we want, so we need to normalize the
    // direction. this is ok as the order won't matter when calculating overlap
    let (s1, e1) = normalize_bounds(a);
    let (s2, e2) = normalize_bounds(b);

    // from this point on, s1, e1, s2, and e2 must be of a value in
    // {Bound::Included(_), Bound::Unbounded}
    debug_assert_matches!(s1, Bound::Included(_) | Bound::Unbounded);
    debug_assert_matches!(e1, Bound::Included(_) | Bound::Unbounded);
    debug_assert_matches!(s2, Bound::Included(_) | Bound::Unbounded);
    debug_assert_matches!(e2, Bound::Included(_) | Bound::Unbounded);

    let (r1, r2) = (
        choose_bound(cmp::max, (s1, s2), (e1, e2)),
        choose_bound(cmp::min, (e1, e2), (s1, s2)),
    );

    match (r1, r2) {
        (Bound::Included(a), Bound::Included(b)) => {
            if a <= b {
                Overlap::Positive(b - a)
            } else {
                Overlap::Negative(a - b)
            }
        }
        (Bound::Unbounded, Bound::Included(b)) => Overlap::NegativeInfinity(b),
        (Bound::Included(a), Bound::Unbounded) => Overlap::PositiveInfinity(a),
        (Bound::Unbounded, Bound::Unbounded) => Overlap::Infinity,
        (Bound::Excluded(_), _) | (_, Bound::Excluded(_)) => unsafe { unreachable_unchecked() },
    }
}
