// Copyright 2006 The Android Open Source Project
// Copyright 2020 Evgeniy Reizner
//
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::{Point, NormalizedF32, Transform};

use crate::checked_geom_ext::TransformExt;
use crate::floating_point::FLOAT_PI;
use crate::scalar::{Scalar, SCALAR_NEARLY_ZERO, SCALAR_ROOT_2_OVER_2};
use crate::wide::F32x2;

mod private {
    use crate::scalar::Scalar;
    use crate::{FiniteF32, NormalizedF32};

    /// A finite f32 in (0,1) range.
    #[derive(Copy, Clone, PartialEq, PartialOrd, Debug)]
    #[repr(transparent)]
    pub struct TValue(FiniteF32);

    impl TValue {
        // Just a random, valid numbers to init the array.
        // Will be overwritten anyway.
        pub const ANY: Self = unsafe { TValue(FiniteF32::new_unchecked(0.5)) };

        pub fn new(n: f32) -> Option<Self> {
            if n > 0.0 && n < 1.0 {
                // `n` is guarantee to be finite after the bounds check
                unsafe { Some(TValue(FiniteF32::new_unchecked(n))) }
            } else {
                None
            }
        }

        pub fn new_bounded(n: f32) -> Self {
            let n = n.bound(std::f32::EPSILON, 1.0 - std::f32::EPSILON);
            // `n` is guarantee to be finite after clamping
            debug_assert!(n.is_finite());
            unsafe { TValue(FiniteF32::new_unchecked(n)) }
        }

        pub fn get(self) -> f32 {
            self.0.get()
        }

        pub fn to_normalized(self) -> NormalizedF32 {
            // TValue is (0,1), while NormalizedF32 is [0,1], so it will always fit.
            unsafe { NormalizedF32::new_unchecked(self.0.get()) }
        }
    }
}
pub use private::TValue;
use crate::path_builder::PathDirection;


/// use for : eval(t) == A * t^2 + B * t + C
#[derive(Default)]
pub struct QuadCoeff {
    a: F32x2,
    b: F32x2,
    c: F32x2,
}

impl QuadCoeff {
    pub fn from_points(points: &[Point; 3]) -> Self {
        let c = points[0].to_f32x2();
        let p1 = points[1].to_f32x2();
        let p2 = points[2].to_f32x2();
        let b = times_2(p1 - c);
        let a = p2 - times_2(p1) + c;

        QuadCoeff { a, b, c }
    }

    pub fn eval(&self, t: F32x2) -> F32x2 {
        (self.a * t + self.b) * t + self.c
    }
}


#[derive(Default)]
pub struct CubicCoeff {
    a: F32x2,
    b: F32x2,
    c: F32x2,
    d: F32x2,
}

impl CubicCoeff {
    pub fn from_points(points: &[Point; 4]) -> Self {
        let p0 = points[0].to_f32x2();
        let p1 = points[1].to_f32x2();
        let p2 = points[2].to_f32x2();
        let p3 = points[3].to_f32x2();
        let three = F32x2::splat(3.0);

        CubicCoeff {
            a: p3 + three * (p1 - p2) - p0,
            b: three * (p2 - times_2(p1) + p0),
            c: three * (p1 - p0),
            d: p0,
        }
    }

    pub fn eval(&self, t: F32x2) -> F32x2 {
        ((self.a * t + self.b) * t + self.c) * t + self.d
    }
}


// TODO: to a custom type?
pub fn new_t_values() -> [TValue; 3] {
    [TValue::ANY, TValue::ANY, TValue::ANY]
}

// TODO: return custom type
/// Returns 0 for 1 quad, and 1 for two quads, either way the answer is stored in dst[].
///
/// Guarantees that the 1/2 quads will be monotonic.
pub fn chop_quad_at_x_extrema(src: &[Point; 3], dst: &mut [Point; 5]) -> usize {
    let a = src[0].x;
    let mut b = src[1].x;
    let c = src[2].x;

    if is_not_monotonic(a, b, c) {
        if let Some(t_value) = valid_unit_divide(a - b, a - b - b + c) {
            chop_quad_at(src, t_value, dst);

            // flatten double quad extrema
            dst[1].x = dst[2].x;
            dst[3].x = dst[2].x;

            return 1;
        }

        // if we get here, we need to force dst to be monotonic, even though
        // we couldn't compute a unit_divide value (probably underflow).
        b = if (a - b).abs() < (b - c).abs() { a } else { c };
    }

    dst[0] = Point::from_xy(a, src[0].y);
    dst[1] = Point::from_xy(b, src[1].y);
    dst[2] = Point::from_xy(c, src[2].y);
    0
}

/// Returns 0 for 1 quad, and 1 for two quads, either way the answer is stored in dst[].
///
/// Guarantees that the 1/2 quads will be monotonic.
pub fn chop_quad_at_y_extrema(src: &[Point; 3], dst: &mut [Point; 5]) -> usize {
    let a = src[0].y;
    let mut b = src[1].y;
    let c = src[2].y;

    if is_not_monotonic(a, b, c) {
        if let Some(t_value) = valid_unit_divide(a - b, a - b - b + c) {
            chop_quad_at(src, t_value, dst);

            // flatten double quad extrema
            dst[1].y = dst[2].y;
            dst[3].y = dst[2].y;

            return 1;
        }

        // if we get here, we need to force dst to be monotonic, even though
        // we couldn't compute a unit_divide value (probably underflow).
        b = if (a - b).abs() < (b - c).abs() { a } else { c };
    }

    dst[0] = Point::from_xy(src[0].x, a);
    dst[1] = Point::from_xy(src[1].x, b);
    dst[2] = Point::from_xy(src[2].x, c);
    0
}

fn is_not_monotonic(a: f32, b: f32, c: f32) -> bool {
    let ab = a - b;
    let mut bc = b - c;
    if ab < 0.0 {
        bc = -bc;
    }

    ab == 0.0 || bc < 0.0
}

pub fn chop_quad_at(src: &[Point], t: TValue, dst: &mut [Point; 5]) {
    let p0 = src[0].to_f32x2();
    let p1 = src[1].to_f32x2();
    let p2 = src[2].to_f32x2();
    let tt = F32x2::splat(t.get());

    let p01 = interp(p0, p1, tt);
    let p12 = interp(p1, p2, tt);

    dst[0] = Point::from_f32x2(p0);
    dst[1] = Point::from_f32x2(p01);
    dst[2] = Point::from_f32x2(interp(p01, p12, tt));
    dst[3] = Point::from_f32x2(p12);
    dst[4] = Point::from_f32x2(p2);
}

pub fn chop_cubic_at_x_extrema(src: &[Point; 4], dst: &mut [Point; 10]) -> usize {
    let mut t_values = new_t_values();
    let t_values = find_cubic_extrema(src[0].x, src[1].x, src[2].x, src[3].x, &mut t_values);

    chop_cubic_at(src, &t_values, dst);
    if !t_values.is_empty() {
        // we do some cleanup to ensure our X extrema are flat
        dst[2].x = dst[3].x;
        dst[4].x = dst[3].x;
        if t_values.len() == 2 {
            dst[5].x = dst[6].x;
            dst[7].x = dst[6].x;
        }
    }

    t_values.len()
}

/// Given 4 points on a cubic bezier, chop it into 1, 2, 3 beziers such that
/// the resulting beziers are monotonic in Y.
///
/// This is called by the scan converter.
///
/// Depending on what is returned, dst[] is treated as follows:
///
/// - 0: dst[0..3] is the original cubic
/// - 1: dst[0..3] and dst[3..6] are the two new cubics
/// - 2: dst[0..3], dst[3..6], dst[6..9] are the three new cubics
pub fn chop_cubic_at_y_extrema(src: &[Point; 4], dst: &mut [Point; 10]) -> usize {
    let mut t_values = new_t_values();
    let t_values = find_cubic_extrema(src[0].y, src[1].y, src[2].y, src[3].y, &mut t_values);

    chop_cubic_at(src, &t_values, dst);
    if !t_values.is_empty() {
        // we do some cleanup to ensure our Y extrema are flat
        dst[2].y = dst[3].y;
        dst[4].y = dst[3].y;
        if t_values.len() == 2 {
            dst[5].y = dst[6].y;
            dst[7].y = dst[6].y;
        }
    }

    t_values.len()
}

// Cubic'(t) = At^2 + Bt + C, where
// A = 3(-a + 3(b - c) + d)
// B = 6(a - 2b + c)
// C = 3(b - a)
// Solve for t, keeping only those that fit between 0 < t < 1
fn find_cubic_extrema(a: f32, b: f32, c: f32, d: f32, t_values: &mut [TValue; 3]) -> &[TValue] {
    // we divide A,B,C by 3 to simplify
    let na = d - a + 3.0 * (b - c);
    let nb = 2.0 * (a - b - b + c);
    let nc = b - a;

    let roots = find_unit_quad_roots(na, nb, nc, t_values);
    &t_values[0..roots]
}

/// From Numerical Recipes in C.
///
/// Q = -1/2 (B + sign(B) sqrt[B*B - 4*A*C])
/// x1 = Q / A
/// x2 = C / Q
pub fn find_unit_quad_roots(a: f32, b: f32, c: f32, roots: &mut [TValue; 3]) -> usize {
    if a == 0.0 {
        if let Some(r) = valid_unit_divide(-c, b) {
            roots[0] = r;
            return 1;
        } else {
            return 0;
        }
    }

    // use doubles so we don't overflow temporarily trying to compute R
    let mut dr = f64::from(b) * f64::from(b) - 4.0 * f64::from(a) * f64::from(c);
    if dr < 0.0 {
        return 0;
    }
    dr = dr.sqrt();
    let r = dr as f32;
    if !r.is_finite() {
        return 0;
    }

    let q = if b < 0.0 { -(b - r) / 2.0 } else { -(b + r) / 2.0 };

    let mut roots_offset = 0;
    if let Some(r) = valid_unit_divide(q, a) {
        roots[roots_offset] = r;
        roots_offset += 1;
    }

    if let Some(r) = valid_unit_divide(c, q) {
        roots[roots_offset] = r;
        roots_offset += 1;
    }

    if roots_offset == 2 {
        if roots[0].get() > roots[1].get() {
            roots.swap(0, 1);
        } else if roots[0] == roots[1] { // nearly-equal?
            roots_offset -= 1; // skip the double root
        }
    }

    roots_offset
}

// http://code.google.com/p/skia/issues/detail?id=32
//
// This test code would fail when we didn't check the return result of
// valid_unit_divide in SkChopCubicAt(... tValues[], int roots). The reason is
// that after the first chop, the parameters to valid_unit_divide are equal
// (thanks to finite float precision and rounding in the subtracts). Thus
// even though the 2nd tValue looks < 1.0, after we renormalize it, we end
// up with 1.0, hence the need to check and just return the last cubic as
// a degenerate clump of 4 points in the same place.
//
// static void test_cubic() {
//     SkPoint src[4] = {
//         { 556.25000, 523.03003 },
//         { 556.23999, 522.96002 },
//         { 556.21997, 522.89001 },
//         { 556.21997, 522.82001 }
//     };
//     SkPoint dst[10];
//     SkScalar tval[] = { 0.33333334f, 0.99999994f };
//     SkChopCubicAt(src, dst, tval, 2);
// }
fn chop_cubic_at(src: &[Point; 4], t_values: &[TValue], dst: &mut [Point]) {
    if t_values.is_empty() {
        // nothing to chop
        dst[0] = src[0];
        dst[1] = src[1];
        dst[2] = src[2];
        dst[3] = src[3];
    } else {
        let mut t = t_values[0];
        let mut tmp = [Point::zero(); 4];

        // Reduce the `src` lifetime, so we can use `src = &tmp` later.
        let mut src = src;

        let mut dst_offset = 0;
        for i in 0..t_values.len() {
            chop_cubic_at2(src, t, &mut dst[dst_offset..]);
            if i == t_values.len() - 1 {
                break;
            }

            dst_offset += 3;
            // have src point to the remaining cubic (after the chop)
            tmp[0] = dst[dst_offset + 0];
            tmp[1] = dst[dst_offset + 1];
            tmp[2] = dst[dst_offset + 2];
            tmp[3] = dst[dst_offset + 3];
            src = &tmp;

            // watch out in case the renormalized t isn't in range
            let n = valid_unit_divide(
                t_values[i+1].get() - t_values[i].get(),
                1.0 - t_values[i].get(),
            );

            match n {
                Some(n) => t = n,
                None => {
                    // if we can't, just create a degenerate cubic
                    dst[dst_offset + 4] = src[3];
                    dst[dst_offset + 5] = src[3];
                    dst[dst_offset + 6] = src[3];
                    break;
                }
            }
        }
    }
}

pub fn chop_cubic_at2(src: &[Point; 4], t: TValue, dst: &mut [Point]) {
    let p0 = src[0].to_f32x2();
    let p1 = src[1].to_f32x2();
    let p2 = src[2].to_f32x2();
    let p3 = src[3].to_f32x2();
    let tt = F32x2::splat(t.get());

    let ab = interp(p0, p1, tt);
    let bc = interp(p1, p2, tt);
    let cd = interp(p2, p3, tt);
    let abc = interp(ab, bc, tt);
    let bcd = interp(bc, cd, tt);
    let abcd = interp(abc, bcd, tt);

    dst[0] = Point::from_f32x2(p0);
    dst[1] = Point::from_f32x2(ab);
    dst[2] = Point::from_f32x2(abc);
    dst[3] = Point::from_f32x2(abcd);
    dst[4] = Point::from_f32x2(bcd);
    dst[5] = Point::from_f32x2(cd);
    dst[6] = Point::from_f32x2(p3);
}

fn valid_unit_divide(mut numer: f32, mut denom: f32) -> Option<TValue> {
    if numer < 0.0 {
        numer = -numer;
        denom = -denom;
    }

    if denom == 0.0 || numer == 0.0 || numer >= denom {
        return None;
    }

    let r = numer / denom;
    TValue::new(r)
}

fn interp(v0: F32x2, v1: F32x2, t: F32x2) -> F32x2 {
    v0 + (v1 - v0) * t
}

fn times_2(value: F32x2) -> F32x2 {
    value + value
}

// F(t)    = a (1 - t) ^ 2 + 2 b t (1 - t) + c t ^ 2
// F'(t)   = 2 (b - a) + 2 (a - 2b + c) t
// F''(t)  = 2 (a - 2b + c)
//
// A = 2 (b - a)
// B = 2 (a - 2b + c)
//
// Maximum curvature for a quadratic means solving
// Fx' Fx'' + Fy' Fy'' = 0
//
// t = - (Ax Bx + Ay By) / (Bx ^ 2 + By ^ 2)
pub fn find_quad_max_curvature(src: &[Point; 3]) -> NormalizedF32 {
    let ax = src[1].x - src[0].x;
    let ay = src[1].y - src[0].y;
    let bx = src[0].x - src[1].x - src[1].x + src[2].x;
    let by = src[0].y - src[1].y - src[1].y + src[2].y;

    let mut numer = -(ax * bx + ay * by);
    let mut denom = bx * bx + by * by;
    if denom < 0.0 {
        numer = -numer;
        denom = -denom;
    }

    if numer <= 0.0 {
        return NormalizedF32::ZERO;
    }

    if numer >= denom {
        // Also catches denom=0
        return NormalizedF32::ONE;
    }

    let t = numer / denom;
    NormalizedF32::new(t).unwrap()
}

pub fn eval_quad_at(src: &[Point; 3], t: NormalizedF32) -> Point {
    Point::from_f32x2(QuadCoeff::from_points(src).eval(F32x2::splat(t.get())))
}

/// Set pt to the point on the src quadratic specified by t.
pub fn eval_quad_at2(src: &[Point; 3], t: NormalizedF32, pt: &mut Point, tangent: &mut Point) {
    *pt = eval_quad_at(src, t);
    *tangent = eval_quad_tangent_at(src, t);
}

fn eval_quad_tangent_at(src: &[Point; 3], tol: NormalizedF32) -> Point {
    // The derivative equation is 2(b - a +(a - 2b +c)t). This returns a
    // zero tangent vector when t is 0 or 1, and the control point is equal
    // to the end point. In this case, use the quad end points to compute the tangent.
    if (tol == NormalizedF32::ZERO && src[0] == src[1]) || (tol == NormalizedF32::ONE && src[1] == src[2]) {
        return src[2] - src[0];
    }

    let p0 = src[0].to_f32x2();
    let p1 = src[1].to_f32x2();
    let p2 = src[2].to_f32x2();

    let b = p1 - p0;
    let a = p2 - p1 - b;
    let t = a * F32x2::splat(tol.get()) + b;

    Point::from_f32x2(t + t)
}

// Looking for F' dot F'' == 0
//
// A = b - a
// B = c - 2b + a
// C = d - 3c + 3b - a
//
// F' = 3Ct^2 + 6Bt + 3A
// F'' = 6Ct + 6B
//
// F' dot F'' -> CCt^3 + 3BCt^2 + (2BB + CA)t + AB
pub fn find_cubic_max_curvature<'a>(src: &[Point; 4], t_values: &'a mut [TValue; 3]) -> &'a [TValue] {
    let raw_src = points_to_f32s!(src, 4);

    let mut coeff_x = formulate_f1_dot_f2(raw_src);
    let coeff_y = formulate_f1_dot_f2(&raw_src[1..]);

    for i in 0..4 {
        coeff_x[i] += coeff_y[i];
    }

    let len = solve_cubic_poly(&coeff_x, t_values);
    &t_values[0..len]
}

// Looking for F' dot F'' == 0
//
// A = b - a
// B = c - 2b + a
// C = d - 3c + 3b - a
//
// F' = 3Ct^2 + 6Bt + 3A
// F'' = 6Ct + 6B
//
// F' dot F'' -> CCt^3 + 3BCt^2 + (2BB + CA)t + AB
fn formulate_f1_dot_f2(src: &[f32]) -> [f32; 4] {
    let a = src[2] - src[0];
    let b = src[4] - 2.0 * src[2] + src[0];
    let c = src[6] + 3.0 * (src[2] - src[4]) - src[0];

    [
        c * c,
        3.0 * b * c,
        2.0 * b * b + c * a,
        a * b,
    ]
}

/// Solve coeff(t) == 0, returning the number of roots that lie withing 0 < t < 1.
/// coeff[0]t^3 + coeff[1]t^2 + coeff[2]t + coeff[3]
///
/// Eliminates repeated roots (so that all t_values are distinct, and are always
/// in increasing order.
fn solve_cubic_poly(coeff: &[f32; 4], t_values: &mut [TValue; 3]) -> usize {
    if coeff[0].is_nearly_zero() {
        // we're just a quadratic
        return find_unit_quad_roots(coeff[1], coeff[2], coeff[3], t_values);
    }

    debug_assert!(coeff[0] != 0.0);

    let inva = coeff[0].invert();
    let a = coeff[1] * inva;
    let b = coeff[2] * inva;
    let c = coeff[3] * inva;

    let q = (a*a - b*3.0) / 9.0;
    let r = (2.0*a*a*a - 9.0*a*b + 27.0*c) / 54.0;

    let q3 = q * q * q;
    let r2_minus_q3 = r * r - q3;
    let adiv3 = a / 3.0;

    if r2_minus_q3 < 0.0 { // we have 3 real roots
        // the divide/root can, due to finite precisions, be slightly outside of -1...1
        let theta = (r / q3.sqrt()).bound(-1.0, 1.0).acos();
        let neg2_root_q = -2.0 * q.sqrt();

        t_values[0] = TValue::new_bounded(neg2_root_q * (theta / 3.0).cos() - adiv3);
        t_values[1] = TValue::new_bounded(neg2_root_q * ((theta + 2.0*FLOAT_PI) / 3.0).cos() - adiv3);
        t_values[2] = TValue::new_bounded(neg2_root_q * ((theta - 2.0*FLOAT_PI) / 3.0).cos() - adiv3);

        // now sort the roots
        sort_array3(t_values);
        collapse_duplicates3(t_values)
    } else { // we have 1 real root
        let mut a = r.abs() + r2_minus_q3.sqrt();
        a = scalar_cube_root(a);
        if r > 0.0 {
            a = -a;
        }

        if a != 0.0 {
            a += q / a;
        }

        t_values[0] = TValue::new_bounded(a - adiv3);
        1
    }
}

fn sort_array3(array: &mut [TValue; 3]) {
    if array[0] > array[1] {
        array.swap(0, 1);
    }

    if array[1] > array[2] {
        array.swap(1, 2);
    }

    if array[0] > array[1] {
        array.swap(0, 1);
    }
}

fn collapse_duplicates3(array: &mut [TValue; 3]) -> usize {
    let mut len = 3;

    if array[1] == array[2] {
        len = 2;
    }

    if array[0] == array[1] {
        len = 1;
    }

    len
}

fn scalar_cube_root(x: f32) -> f32 {
    x.powf(0.3333333)
}

pub fn eval_cubic_at(
    src: &[Point; 4],
    t: NormalizedF32,
    loc: Option<&mut Point>,
    tangent: Option<&mut Point>,
    curvature: Option<&mut Point>,
) {
    if let Some(loc) = loc {
        *loc = Point::from_f32x2(CubicCoeff::from_points(src).eval(F32x2::splat(t.get())));
    }

    if let Some(tangent) = tangent {
        // The derivative equation returns a zero tangent vector when t is 0 or 1, and the
        // adjacent control point is equal to the end point. In this case, use the
        // next control point or the end points to compute the tangent.
        if (t.get() == 0.0 && src[0] == src[1]) || (t.get() == 1.0 && src[2] == src[3]) {
            if t.get() == 0.0 {
                *tangent = src[2] - src[0];
            } else {
                *tangent = src[3] - src[1];
            }

            if tangent.x != 0.0 && tangent.y != 0.0 {
                *tangent = src[3] - src[0];
            }
        } else {
            *tangent = eval_cubic_derivative(src, t);
        }
    }

    if let Some(curvature) = curvature {
        *curvature = eval_cubic_2nd_derivative(src, t);
    }
}

fn eval_cubic_derivative(src: &[Point; 4], t: NormalizedF32) -> Point {
    let p0 = src[0].to_f32x2();
    let p1 = src[1].to_f32x2();
    let p2 = src[2].to_f32x2();
    let p3 = src[3].to_f32x2();

    let coeff = QuadCoeff {
        a: p3 + F32x2::splat(3.0) * (p1 - p2) - p0,
        b: times_2(p2 - times_2(p1) + p0),
        c: p1 - p0,
    };

    Point::from_f32x2(coeff.eval(F32x2::splat(t.get())))
}

fn eval_cubic_2nd_derivative(src: &[Point; 4], t: NormalizedF32) -> Point {
    let p0 = src[0].to_f32x2();
    let p1 = src[1].to_f32x2();
    let p2 = src[2].to_f32x2();
    let p3 = src[3].to_f32x2();
    let a = p3 + F32x2::splat(3.0) * (p1 - p2) - p0;
    let b = p2 - times_2(p1) + p0;

    Point::from_f32x2(a * F32x2::splat(t.get()) + b)
}

// http://www.faculty.idc.ac.il/arik/quality/appendixA.html
//
// Inflection means that curvature is zero.
// Curvature is [F' x F''] / [F'^3]
// So we solve F'x X F''y - F'y X F''y == 0
// After some canceling of the cubic term, we get
// A = b - a
// B = c - 2b + a
// C = d - 3c + 3b - a
// (BxCy - ByCx)t^2 + (AxCy - AyCx)t + AxBy - AyBx == 0
pub fn find_cubic_inflections<'a>(src: &[Point; 4], t_values: &'a mut [TValue; 3]) -> &'a [TValue] {
    let ax = src[1].x - src[0].x;
    let ay = src[1].y - src[0].y;
    let bx = src[2].x - 2.0 * src[1].x + src[0].x;
    let by = src[2].y - 2.0 * src[1].y + src[0].y;
    let cx = src[3].x + 3.0 * (src[1].x - src[2].x) - src[0].x;
    let cy = src[3].y + 3.0 * (src[1].y - src[2].y) - src[0].y;

    let len = find_unit_quad_roots(
        bx * cy - by * cx,
        ax * cy - ay * cx,
        ax * by - ay * bx,
        t_values,
    );

    &t_values[0..len]
}

// Return location (in t) of cubic cusp, if there is one.
// Note that classify cubic code does not reliably return all cusp'd cubics, so
// it is not called here.
pub fn find_cubic_cusp(src: &[Point; 4]) -> Option<TValue> {
    // When the adjacent control point matches the end point, it behaves as if
    // the cubic has a cusp: there's a point of max curvature where the derivative
    // goes to zero. Ideally, this would be where t is zero or one, but math
    // error makes not so. It is not uncommon to create cubics this way; skip them.
    if src[0] == src[1] {
        return None;
    }

    if src[2] == src[3] {
        return None;
    }

    // Cubics only have a cusp if the line segments formed by the control and end points cross.
    // Detect crossing if line ends are on opposite sides of plane formed by the other line.
    if on_same_side(src, 0, 2) || on_same_side(src, 2, 0) {
        return None;
    }

    // Cubics may have multiple points of maximum curvature, although at most only
    // one is a cusp.
    let mut t_values = new_t_values();
    let max_curvature = find_cubic_max_curvature(src, &mut t_values);
    for test_t in max_curvature {
        // A cusp is at the max curvature, and also has a derivative close to zero.
        // Choose the 'close to zero' meaning by comparing the derivative length
        // with the overall cubic size.
        let d_pt = eval_cubic_derivative(src, test_t.to_normalized());
        let d_pt_magnitude = d_pt.length_sqd();
        let precision = calc_cubic_precision(src);
        if d_pt_magnitude < precision {
            // All three max curvature t values may be close to the cusp;
            // return the first one.
            return Some(*test_t);
        }
    }

    None
}

// Returns true if both points src[testIndex], src[testIndex+1] are in the same half plane defined
// by the line segment src[lineIndex], src[lineIndex+1].
fn on_same_side(src: &[Point; 4], test_index: usize, line_index: usize) -> bool {
    let origin = src[line_index];
    let line = src[line_index + 1] - origin;
    let mut crosses = [0.0, 0.0];
    for index in 0..2 {
        let test_line = src[test_index + index] - origin;
        crosses[index] = line.cross(test_line);
    }

    crosses[0] * crosses[1] >= 0.0
}

// Returns a constant proportional to the dimensions of the cubic.
// Constant found through experimentation -- maybe there's a better way....
fn calc_cubic_precision(src: &[Point; 4]) -> f32 {
    (  src[1].distance_to_sqd(src[0])
     + src[2].distance_to_sqd(src[1])
     + src[3].distance_to_sqd(src[2])) * 1e-8
}


#[derive(Copy, Clone, Default)]
pub struct Conic {
    pub points: [Point; 3],
    pub weight: f32,
}

impl Conic {
    pub fn new(pt0: Point, pt1: Point, pt2: Point, weight: f32) -> Self {
        Conic {
            points: [pt0, pt1, pt2],
            weight,
        }
    }

    pub fn from_points(points: &[Point], weight: f32) -> Self {
        Conic {
            points: [points[0], points[1], points[2]],
            weight,
        }
    }

    // Chop this conic into N quads, stored continuously in pts[], where
    // N = 1 << pow2. The amount of storage needed is (1 + 2 * N)
    pub fn chop_into_quads_pow2(&self, pow2: u8, points: &mut [Point]) -> usize {
        // Skia allow 0..5, but we allow only 3.
        // We can simply the code a lot by not supporting pow2==5
        debug_assert!(pow2 == 3);

        points[0] = self.points[0];
        subdivide(self, &mut points[1..], pow2);

        let quad_count = 1 << pow2;
        let pt_count = 2 * quad_count + 1;
        if points.iter().take(pt_count).any(|n| !n.is_finite()) {
            // if we generated a non-finite, pin ourselves to the middle of the hull,
            // as our first and last are already on the first/last pts of the hull.
            for i in 1..pt_count-1 {
                points[i] = self.points[1];
            }
        }

        1 << pow2
    }

    fn chop(&self) -> (Conic, Conic) {
        let scale = F32x2::splat((1.0 + self.weight).invert());
        let new_w = subdivide_weight_value(self.weight);

        let p0 = self.points[0].to_f32x2();
        let p1 = self.points[1].to_f32x2();
        let p2 = self.points[2].to_f32x2();
        let ww = F32x2::splat(self.weight);

        let wp1 = ww * p1;
        let m = (p0 + times_2(wp1) + p2) * scale * F32x2::splat(0.5);
        let mut m_pt = Point::from_f32x2(m);
        if !m_pt.is_finite() {
            let w_d = self.weight as f64;
            let w_2 = w_d * 2.0;
            let scale_half = 1.0 / (1.0 + w_d) * 0.5;
            m_pt.x = ((self.points[0].x as f64 + w_2 * self.points[1].x as f64 + self.points[2].x as f64) * scale_half) as f32;
            m_pt.y = ((self.points[0].y as f64 + w_2 * self.points[1].y as f64 + self.points[2].y as f64) * scale_half) as f32;
        }

        (
            Conic {
                points: [self.points[0], Point::from_f32x2((p0 + wp1) * scale), m_pt],
                weight: new_w,
            },
            Conic {
                points: [m_pt, Point::from_f32x2((wp1 + p2) * scale), self.points[2]],
                weight: new_w,
            },
        )
    }

    pub fn build_unit_arc<'a>(
        u_start: Point,
        u_stop: Point,
        dir: PathDirection,
        user_transform: &Transform,
        dst: &'a mut [Conic; 5],
    ) -> &'a [Conic] {
        // rotate by x,y so that u_start is (1.0)
        let x = u_start.dot(u_stop);
        let mut y = u_start.cross(u_stop);

        let abs_y = y.abs();

        // check for (effectively) coincident vectors
        // this can happen if our angle is nearly 0 or nearly 180 (y == 0)
        // ... we use the dot-prod to distinguish between 0 and 180 (x > 0)
        if  abs_y <= SCALAR_NEARLY_ZERO &&
            x > 0.0 &&
            ((y >= 0.0 && dir == PathDirection::CW) || (y <= 0.0 && dir == PathDirection::CCW))
        {
            return &[];
        }

        if dir == PathDirection::CCW {
            y = -y;
        }

        // We decide to use 1-conic per quadrant of a circle. What quadrant does [xy] lie in?
        //      0 == [0  .. 90)
        //      1 == [90 ..180)
        //      2 == [180..270)
        //      3 == [270..360)
        //
        let mut quadrant = 0;
        if y == 0.0 {
            quadrant = 2; // 180
            debug_assert!((x + 1.0) <= SCALAR_NEARLY_ZERO);
        } else if x == 0.0 {
            debug_assert!(abs_y - 1.0 <= SCALAR_NEARLY_ZERO);
            quadrant = if y > 0.0 { 1 } else { 3 }; // 90 / 270
        } else {
            if y < 0.0 {
                quadrant += 2;
            }

            if (x < 0.0) != (y < 0.0) {
                quadrant += 1;
            }
        }

        let quadrant_points = [
            Point::from_xy(1.0, 0.0),
            Point::from_xy(1.0, 1.0),
            Point::from_xy(0.0, 1.0),
            Point::from_xy(-1.0, 1.0),
            Point::from_xy(-1.0, 0.0),
            Point::from_xy(-1.0, -1.0),
            Point::from_xy(0.0, -1.0),
            Point::from_xy(1.0, -1.0),
        ];

        const QUADRANT_WEIGHT: f32 = SCALAR_ROOT_2_OVER_2;

        let mut conic_count = quadrant;
        for i in 0..conic_count {
            dst[i] = Conic::from_points(&quadrant_points[i * 2..], QUADRANT_WEIGHT);
        }

        // Now compute any remaing (sub-90-degree) arc for the last conic
        let final_pt = Point::from_xy(x, y);
        let last_q = quadrant_points[quadrant * 2];  // will already be a unit-vector
        let dot = last_q.dot(final_pt);
        debug_assert!(0.0 <= dot && dot <= 1.0 + SCALAR_NEARLY_ZERO);

        if dot < 1.0 {
            let mut off_curve = Point::from_xy(last_q.x + x, last_q.y + y);
            // compute the bisector vector, and then rescale to be the off-curve point.
            // we compute its length from cos(theta/2) = length / 1, using half-angle identity we get
            // length = sqrt(2 / (1 + cos(theta)). We already have cos() when to computed the dot.
            // This is nice, since our computed weight is cos(theta/2) as well!
            //
            let cos_theta_over_2 = ((1.0 + dot) / 2.0).sqrt();
            off_curve.set_length(cos_theta_over_2.invert());
            if !last_q.almost_equal(off_curve) {
                dst[conic_count] = Conic::new(last_q, off_curve, final_pt, cos_theta_over_2);
                conic_count += 1;
            }
        }

        // now handle counter-clockwise and the initial unitStart rotation
        let mut transform = Transform::from_sin_cos(u_start.y, u_start.x).unwrap();
        if dir == PathDirection::CCW {
            transform.pre_scale(1.0, -1.0);
        }

        transform.post_concat(user_transform);

        for conic in dst.iter_mut().take(conic_count) {
            transform.map_points(&mut conic.points);
        }

        &dst[0..conic_count]
    }
}

fn subdivide_weight_value(w: f32) -> f32 {
    (0.5 + w * 0.5).sqrt()
}

fn subdivide<'a>(src: &Conic, mut points: &'a mut [Point], mut level: u8) -> &'a mut [Point] {
    if level == 0 {
        points[0] = src.points[1];
        points[1] = src.points[2];
        &mut points[2..]
    } else {
        let mut dst = src.chop();

        let start_y = src.points[0].y;
        let end_y = src.points[2].y;
        if between(start_y, src.points[1].y, end_y) {
            // If the input is monotonic and the output is not, the scan converter hangs.
            // Ensure that the chopped conics maintain their y-order.
            let mid_y = dst.0.points[2].y;
            if !between(start_y, mid_y, end_y) {
                // If the computed midpoint is outside the ends, move it to the closer one.
                let closer_y = if (mid_y - start_y).abs() < (mid_y - end_y).abs() { start_y } else { end_y };
                dst.0.points[2].y = closer_y;
                dst.1.points[0].y = closer_y;
            }

            if !between(start_y, dst.0.points[1].y, dst.0.points[2].y) {
                // If the 1st control is not between the start and end, put it at the start.
                // This also reduces the quad to a line.
                dst.0.points[1].y = start_y;
            }

            if !between(dst.1.points[0].y, dst.1.points[1].y, end_y) {
                // If the 2nd control is not between the start and end, put it at the end.
                // This also reduces the quad to a line.
                dst.1.points[1].y = end_y;
            }

            // Verify that all five points are in order.
            debug_assert!(between(start_y, dst.0.points[1].y, dst.0.points[2].y));
            debug_assert!(between(dst.0.points[1].y, dst.0.points[2].y, dst.1.points[1].y));
            debug_assert!(between(dst.0.points[2].y, dst.1.points[1].y, end_y));
        }

        level -= 1;
        points = subdivide(&dst.0, points, level);
        subdivide(&dst.1, points, level)
    }
}

// This was originally developed and tested for pathops: see SkOpTypes.h
// returns true if (a <= b <= c) || (a >= b >= c)
fn between(a: f32, b: f32, c: f32) -> bool {
    (a - b) * (c - b) <= 0.0
}

pub fn chop_mono_cubic_at_x(src: &[Point; 4], x: f32, dst: &mut [Point; 7]) -> bool {
    cubic_dchop_at_intercept(src, x, true, dst)
}

pub fn chop_mono_cubic_at_y(src: &[Point; 4], y: f32, dst: &mut [Point; 7]) -> bool {
    cubic_dchop_at_intercept(src, y, false, dst)
}

fn cubic_dchop_at_intercept(src: &[Point; 4], intercept: f32, is_vertical: bool, dst: &mut [Point; 7]) -> bool {
    use crate::path_ops::{cubic64::Cubic64, point64::Point64, line_cubic_intersections};

    let src = [
        Point64::from_point(src[0]),
        Point64::from_point(src[1]),
        Point64::from_point(src[2]),
        Point64::from_point(src[3]),
    ];

    let cubic = Cubic64::new(src);
    let mut roots = [0.0; 3];
    let count = if is_vertical {
        line_cubic_intersections::vertical_intersect(&cubic, f64::from(intercept), &mut roots)
    } else {
        line_cubic_intersections::horizontal_intersect(&cubic, f64::from(intercept), &mut roots)
    };

    if count > 0 {
        let pair = cubic.chop_at(roots[0]);
        for i in 0..7 {
            dst[i] = pair.points[i].to_point();
        }

        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chop_cubic_at_y_extrema_1() {
        let src = [
            Point::from_xy(10.0, 20.0),
            Point::from_xy(67.0, 437.0),
            Point::from_xy(298.0, 213.0),
            Point::from_xy(401.0, 214.0),
        ];

        let mut dst = [Point::zero(); 10];
        let n = chop_cubic_at_y_extrema(&src, &mut dst);
        assert_eq!(n, 2);
        assert_eq!(dst[0], Point::from_xy(10.0, 20.0));
        assert_eq!(dst[1], Point::from_xy(37.508274, 221.24475));
        assert_eq!(dst[2], Point::from_xy(105.541855, 273.19803));
        assert_eq!(dst[3], Point::from_xy(180.15599, 273.19803));
        assert_eq!(dst[4], Point::from_xy(259.80502, 273.19803));
        assert_eq!(dst[5], Point::from_xy(346.9527, 213.99666));
        assert_eq!(dst[6], Point::from_xy(400.30844, 213.99666));
        assert_eq!(dst[7], Point::from_xy(400.53958, 213.99666));
        assert_eq!(dst[8], Point::from_xy(400.7701, 213.99777));
        assert_eq!(dst[9], Point::from_xy(401.0, 214.0));
    }
}