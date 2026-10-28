// sin/cos exactly as V8 computes Math.sin/Math.cos (src/base/ieee754.cc),
// so rendered audio matches the JS engine bit for bit. The FreeBSD/musl
// variants (and the libm crate) use a different __kernel_cos and disagree in
// the last bit for about 1% of arguments.
//
// The following is adapted from fdlibm (http://www.netlib.org/fdlibm).
//
// ====================================================
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunSoft, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
// ====================================================
//
// The original source code covered by the above license above has been
// modified significantly by Google Inc.
// Copyright 2016 the V8 project authors. All rights reserved.
// (V8 is distributed under a BSD-style license.)

// Constants and idioms are kept verbatim from fdlibm (`x - x` makes NaN).
#![allow(clippy::approx_constant, clippy::eq_op, clippy::excessive_precision, clippy::explicit_counter_loop)]

// Node on Apple Silicon (macOS arm64) is compiled with Apple clang's default
// floating-point contraction, which fuses V8's `a*b + c` expressions into
// single-rounding FMA instructions; on x86_64 (Windows, Linux, Intel macOS)
// nothing is fused. To match Node on each platform, every such expression in
// V8's ieee754.cc goes through `mad`: fused on Apple arm64, and otherwise
// exactly the unfused `a*b + c` (the rewrites, like `t - f*P` as
// `(-f)*P + t`, are exact).
const FUSED: bool = cfg!(all(target_arch = "aarch64", target_vendor = "apple"));

#[inline(always)]
fn mad(a: f64, b: f64, c: f64) -> f64 {
    if FUSED { a.mul_add(b, c) } else { a * b + c }
}

fn high_word(x: f64) -> i32 {
    (x.to_bits() >> 32) as u32 as i32
}

fn low_word(x: f64) -> u32 {
    x.to_bits() as u32
}

fn insert_words(hi: i32, lo: u32) -> f64 {
    f64::from_bits((u64::from(hi as u32) << 32) | u64::from(lo))
}

fn set_high_word(d: f64, v: i32) -> f64 {
    f64::from_bits((d.to_bits() & 0x0000_0000_FFFF_FFFF) | (u64::from(v as u32) << 32))
}

fn set_low_word(d: f64, v: u32) -> f64 {
    f64::from_bits((d.to_bits() & 0xFFFF_FFFF_0000_0000) | u64::from(v))
}

/*
 * Table of constants for 2/pi, 396 Hex digits (476 decimal) of 2/pi
 */
const TWO_OVER_PI: [i32; 66] = [
    0xA2F983, 0x6E4E44, 0x1529FC, 0x2757D1, 0xF534DD, 0xC0DB62, 0x95993C,
    0x439041, 0xFE5163, 0xABDEBB, 0xC561B7, 0x246E3A, 0x424DD2, 0xE00649,
    0x2EEA09, 0xD1921C, 0xFE1DEB, 0x1CB129, 0xA73EE8, 0x8235F5, 0x2EBB44,
    0x84E99C, 0x7026B4, 0x5F7E41, 0x3991D6, 0x398353, 0x39F49C, 0x845F8B,
    0xBDF928, 0x3B1FF8, 0x97FFDE, 0x05980F, 0xEF2F11, 0x8B5A0A, 0x6D1F6D,
    0x367ECF, 0x27CB09, 0xB74F46, 0x3F669E, 0x5FEA2D, 0x7527BA, 0xC7EBE5,
    0xF17B3D, 0x0739F7, 0x8A5292, 0xEA6BFB, 0x5FB11F, 0x8D5D08, 0x560330,
    0x46FC7B, 0x6BABF0, 0xCFBC20, 0x9AF436, 0x1DA9E3, 0x91615E, 0xE61B08,
    0x659985, 0x5F14A0, 0x68408D, 0xFFD880, 0x4D7327, 0x310606, 0x1556CA,
    0x73A8C9, 0x60E27B, 0xC08C6B,
];

const NPIO2_HW: [i32; 32] = [
    0x3FF921FB, 0x400921FB, 0x4012D97C, 0x401921FB, 0x401F6A7A, 0x4022D97C,
    0x4025FDBB, 0x402921FB, 0x402C463A, 0x402F6A7A, 0x4031475C, 0x4032D97C,
    0x40346B9C, 0x4035FDBB, 0x40378FDB, 0x403921FB, 0x403AB41B, 0x403C463A,
    0x403DD85A, 0x403F6A7A, 0x40407E4C, 0x4041475C, 0x4042106C, 0x4042D97C,
    0x4043A28C, 0x40446B9C, 0x404534AC, 0x4045FDBB, 0x4046C6CB, 0x40478FDB,
    0x404858EB, 0x404921FB,
];

/*
 * invpio2:  53 bits of 2/pi
 * pio2_1:   first  33 bit of pi/2
 * pio2_1t:  pi/2 - pio2_1
 * pio2_2:   second 33 bit of pi/2
 * pio2_2t:  pi/2 - (pio2_1+pio2_2)
 * pio2_3:   third  33 bit of pi/2
 * pio2_3t:  pi/2 - (pio2_1+pio2_2+pio2_3)
 */
const HALF: f64 = 5.00000000000000000000e-01; /* 0x3FE00000, 0x00000000 */
const TWO24: f64 = 1.67772160000000000000e+07; /* 0x41700000, 0x00000000 */
const TWON24: f64 = 5.96046447753906250000e-08; /* 0x3E700000, 0x00000000 */
const INVPIO2: f64 = 6.36619772367581382433e-01; /* 0x3FE45F30, 0x6DC9C883 */
const PIO2_1: f64 = 1.57079632673412561417e+00; /* 0x3FF921FB, 0x54400000 */
const PIO2_1T: f64 = 6.07710050650619224932e-11; /* 0x3DD0B461, 0x1A626331 */
const PIO2_2: f64 = 6.07710050630396597660e-11; /* 0x3DD0B461, 0x1A600000 */
const PIO2_2T: f64 = 2.02226624879595063154e-21; /* 0x3BA3198A, 0x2E037073 */
const PIO2_3: f64 = 2.02226624871116645580e-21; /* 0x3BA3198A, 0x2E000000 */
const PIO2_3T: f64 = 8.47842766036889956997e-32; /* 0x397B839A, 0x252049C1 */

/* __ieee754_rem_pio2(x,y)
 *
 * return the remainder of x rem pi/2 in y[0]+y[1]
 * use __kernel_rem_pio2()
 */
fn rem_pio2(x: f64) -> (i32, f64, f64) {
    let hx = high_word(x); /* high word of x */
    let ix = hx & 0x7FFFFFFF;
    if ix <= 0x3FE921FB {
        /* |x| ~<= pi/4 , no need for reduction */
        return (0, x, 0.0);
    }
    if ix < 0x4002D97C {
        /* |x| < 3pi/4, special case with n=+-1 */
        if hx > 0 {
            let mut z = x - PIO2_1;
            if ix != 0x3FF921FB {
                /* 33+53 bit pi is good enough */
                let y0 = z - PIO2_1T;
                return (1, y0, (z - y0) - PIO2_1T);
            }
            /* near pi/2, use 33+33+53 bit pi */
            z -= PIO2_2;
            let y0 = z - PIO2_2T;
            return (1, y0, (z - y0) - PIO2_2T);
        }
        /* negative x */
        let mut z = x + PIO2_1;
        if ix != 0x3FF921FB {
            /* 33+53 bit pi is good enough */
            let y0 = z + PIO2_1T;
            return (-1, y0, (z - y0) + PIO2_1T);
        }
        /* near pi/2, use 33+33+53 bit pi */
        z += PIO2_2;
        let y0 = z + PIO2_2T;
        return (-1, y0, (z - y0) + PIO2_2T);
    }
    if ix <= 0x413921FB {
        /* |x| ~<= 2^19*(pi/2), medium size */
        let t = x.abs();
        let n = mad(t, INVPIO2, HALF) as i32;
        let f_n = f64::from(n);
        let mut r = mad(-f_n, PIO2_1, t);
        let mut w = f_n * PIO2_1T; /* 1st round good to 85 bit */
        let mut y0;
        if n < 32 && ix != NPIO2_HW[(n - 1) as usize] {
            y0 = r - w; /* quick check no cancellation */
        } else {
            let j = ix >> 20;
            y0 = r - w;
            let high = high_word(y0) as u32;
            let i = j - ((high >> 20) & 0x7FF) as i32;
            if i > 16 {
                /* 2nd iteration needed, good to 118 */
                let t = r;
                w = f_n * PIO2_2;
                r = t - w;
                w = mad(f_n, PIO2_2T, -((t - r) - w));
                y0 = r - w;
                let high = high_word(y0) as u32;
                let i = j - ((high >> 20) & 0x7FF) as i32;
                if i > 49 {
                    /* 3rd iteration need, 151 bits acc */
                    let t = r; /* will cover all possible cases */
                    w = f_n * PIO2_3;
                    r = t - w;
                    w = mad(f_n, PIO2_3T, -((t - r) - w));
                    y0 = r - w;
                }
            }
        }
        let y1 = (r - y0) - w;
        return if hx < 0 { (-n, -y0, -y1) } else { (n, y0, y1) };
    }
    /*
     * all other (large) arguments
     */
    if ix >= 0x7FF00000 {
        /* x is inf or NaN */
        let v = x - x;
        return (0, v, v);
    }
    /* set z = scalbn(|x|,ilogb(x)-23) */
    let mut z = set_low_word(0.0, low_word(x));
    let e0 = (ix >> 20) - 1046; /* e0 = ilogb(z)-23; */
    z = set_high_word(z, ix.wrapping_sub(((e0 as u32) << 20) as i32));
    let mut tx = [0.0f64; 3];
    for t in tx.iter_mut().take(2) {
        *t = f64::from(z as i32);
        z = (z - *t) * TWO24;
    }
    tx[2] = z;
    let mut nx = 3;
    while tx[nx - 1] == 0.0 {
        nx -= 1; /* skip zero term */
    }
    let (n, y0, y1) = kernel_rem_pio2(&tx[..nx], e0);
    if hx < 0 { (-n, -y0, -y1) } else { (n, y0, y1) }
}

/* __kernel_rem_pio2(x,y,e0,nx,prec,ipio2), specialized to prec = 2 (the
 * only precision rem_pio2 uses) and ipio2 = TWO_OVER_PI.
 *
 * __kernel_rem_pio2 return the last three digits of N with
 *              y = x - N*pi/2
 * so that |y| < pi/2.
 *
 * See the V8/fdlibm source for the full description of the method.
 */
fn kernel_rem_pio2(x: &[f64], e0: i32) -> (i32, f64, f64) {
    const PIO2: [f64; 8] = [
        1.57079625129699707031e+00, /* 0x3FF921FB, 0x40000000 */
        7.54978941586159635335e-08, /* 0x3E74442D, 0x00000000 */
        5.39030252995776476554e-15, /* 0x3CF84698, 0x80000000 */
        3.28200341580791294123e-22, /* 0x3B78CC51, 0x60000000 */
        1.27065575308067607349e-29, /* 0x39F01B83, 0x80000000 */
        1.22933308981111328932e-36, /* 0x387A2520, 0x40000000 */
        2.73370053816464559624e-44, /* 0x36E38222, 0x80000000 */
        2.16741683877804819444e-51, /* 0x3569F31D, 0x00000000 */
    ];
    let ipio2 = &TWO_OVER_PI;
    let nx = x.len() as i32;
    let at = |i: i32| i as usize;

    /* initialize jk*/
    let jk: i32 = 4; /* init_jk[prec = 2] */
    let jp = jk;

    /* determine jx,jv,q0, note that 3>q0 */
    let jx = nx - 1;
    let mut jv = (e0 - 3) / 24;
    if jv < 0 {
        jv = 0;
    }
    let mut q0 = e0 - 24 * (jv + 1);

    let mut f = [0.0f64; 20];
    let mut q = [0.0f64; 20];
    let mut fq = [0.0f64; 20];
    let mut iq = [0i32; 20];

    /* set up f[0] to f[jx+jk] where f[jx+jk] = ipio2[jv+jk] */
    let mut j = jv - jx;
    let m = jx + jk;
    for i in 0..=m {
        f[at(i)] = if j < 0 { 0.0 } else { f64::from(ipio2[at(j)]) };
        j += 1;
    }

    /* compute q[0],q[1],...q[jk] */
    for i in 0..=jk {
        let mut fw = 0.0;
        for j in 0..=jx {
            fw = mad(x[at(j)], f[at(jx + i - j)], fw);
        }
        q[at(i)] = fw;
    }

    let mut jz = jk;
    let mut z;
    let mut n;
    let mut ih;
    loop {
        /* distill q[] into iq[] reversingly */
        let mut i = 0;
        let mut j = jz;
        z = q[at(jz)];
        while j > 0 {
            let fw = f64::from((TWON24 * z) as i32);
            iq[at(i)] = (z - TWO24 * fw) as i32;
            z = q[at(j - 1)] + fw;
            i += 1;
            j -= 1;
        }

        /* compute n */
        z = libm::scalbn(z, q0); /* actual value of z */
        z -= 8.0 * (z * 0.125).floor(); /* trim off integer >= 8 */
        n = z as i32;
        z -= f64::from(n);
        ih = 0;
        if q0 > 0 {
            /* need iq[jz-1] to determine n */
            let i = iq[at(jz - 1)] >> (24 - q0);
            n += i;
            iq[at(jz - 1)] -= i << (24 - q0);
            ih = iq[at(jz - 1)] >> (23 - q0);
        } else if q0 == 0 {
            ih = iq[at(jz - 1)] >> 23;
        } else if z >= 0.5 {
            ih = 2;
        }

        if ih > 0 {
            /* q > 0.5 */
            n += 1;
            let mut carry = 0;
            for i in 0..jz {
                /* compute 1-q */
                let j = iq[at(i)];
                if carry == 0 {
                    if j != 0 {
                        carry = 1;
                        iq[at(i)] = 0x1000000 - j;
                    }
                } else {
                    iq[at(i)] = 0xFFFFFF - j;
                }
            }
            if q0 > 0 {
                /* rare case: chance is 1 in 12 */
                match q0 {
                    1 => iq[at(jz - 1)] &= 0x7FFFFF,
                    2 => iq[at(jz - 1)] &= 0x3FFFFF,
                    _ => {}
                }
            }
            if ih == 2 {
                z = 1.0 - z;
                if carry != 0 {
                    z -= libm::scalbn(1.0, q0);
                }
            }
        }

        /* check if recomputation is needed */
        if z == 0.0 {
            let mut j = 0;
            let mut i = jz - 1;
            while i >= jk {
                j |= iq[at(i)];
                i -= 1;
            }
            if j == 0 {
                /* need recomputation */
                let mut k = 1;
                while jk >= k && iq[at(jk - k)] == 0 {
                    k += 1; /* k = no. of terms needed */
                }
                for i in (jz + 1)..=(jz + k) {
                    /* add q[jz+1] to q[jz+k] */
                    f[at(jx + i)] = f64::from(ipio2[at(jv + i)]);
                    let mut fw = 0.0;
                    for j in 0..=jx {
                        fw = mad(x[at(j)], f[at(jx + i - j)], fw);
                    }
                    q[at(i)] = fw;
                }
                jz += k;
                continue;
            }
        }
        break;
    }

    /* chop off zero terms */
    if z == 0.0 {
        jz -= 1;
        q0 -= 24;
        while iq[at(jz)] == 0 {
            jz -= 1;
            q0 -= 24;
        }
    } else {
        /* break z into 24-bit if necessary */
        z = libm::scalbn(z, -q0);
        if z >= TWO24 {
            let fw = f64::from((TWON24 * z) as i32);
            iq[at(jz)] = (z - TWO24 * fw) as i32;
            jz += 1;
            q0 += 24;
            iq[at(jz)] = fw as i32;
        } else {
            iq[at(jz)] = z as i32;
        }
    }

    /* convert integer "bit" chunk to floating-point value */
    let mut fw = libm::scalbn(1.0, q0);
    let mut i = jz;
    while i >= 0 {
        q[at(i)] = fw * f64::from(iq[at(i)]);
        fw *= TWON24;
        i -= 1;
    }

    /* compute PIo2[0,...,jp]*q[jz,...,0] */
    let mut i = jz;
    while i >= 0 {
        let mut fw = 0.0;
        let mut k = 0;
        while k <= jp && k <= jz - i {
            fw = mad(PIO2[at(k)], q[at(i + k)], fw);
            k += 1;
        }
        fq[at(jz - i)] = fw;
        i -= 1;
    }

    /* compress fq[] into y[] */
    let mut fw = 0.0;
    let mut i = jz;
    while i >= 0 {
        fw += fq[at(i)];
        i -= 1;
    }
    let y0 = if ih == 0 { fw } else { -fw };
    fw = fq[0] - fw;
    for i in 1..=jz {
        fw += fq[at(i)];
    }
    let y1 = if ih == 0 { fw } else { -fw };
    (n & 7, y0, y1)
}

/* __kernel_cos( x,  y )
 * kernel cos function on [-pi/4, pi/4], pi/4 ~ 0.785398164
 * Input x is assumed to be bounded by ~pi/4 in magnitude.
 * Input y is the tail of x.
 *
 * For better accuracy when x > 0.3, let qx = |x|/4 with the last 32 bits
 * mask off, and if x > 0.78125, let qx = 0.28125. Then
 *      cos(x+y) = (1-qx) - ((x*x/2-qx) - (r-x*y)).
 */
fn kernel_cos(x: f64, y: f64) -> f64 {
    const ONE: f64 = 1.00000000000000000000e+00; /* 0x3FF00000, 0x00000000 */
    const C1: f64 = 4.16666666666666019037e-02; /* 0x3FA55555, 0x5555554C */
    const C2: f64 = -1.38888888888741095749e-03; /* 0xBF56C16C, 0x16C15177 */
    const C3: f64 = 2.48015872894767294178e-05; /* 0x3EFA01A0, 0x19CB1590 */
    const C4: f64 = -2.75573143513906633035e-07; /* 0xBE927E4F, 0x809C52AD */
    const C5: f64 = 2.08757232129817482790e-09; /* 0x3E21EE9E, 0xBDB4B1C4 */
    const C6: f64 = -1.13596475577881948265e-11; /* 0xBDA8FAE9, 0xBE8838D4 */

    let ix = high_word(x) & 0x7FFFFFFF; /* ix = |x|'s high word*/
    if ix < 0x3E400000 && x as i32 == 0 {
        /* if x < 2**27 */
        return ONE; /* generate inexact */
    }
    let z = x * x;
    let r = z * mad(z, mad(z, mad(z, mad(z, mad(z, C6, C5), C4), C3), C2), C1);
    if ix < 0x3FD33333 {
        /* if |x| < 0.3 */
        return ONE - mad(0.5, z, -mad(z, r, -(x * y)));
    }
    let qx = if ix > 0x3FE90000 {
        /* x > 0.78125 */
        0.28125
    } else {
        insert_words(ix - 0x00200000, 0) /* x/4 */
    };
    let iz = mad(0.5, z, -qx);
    let a = ONE - qx;
    a - (iz - mad(z, r, -(x * y)))
}

/* __kernel_sin( x, y, iy)
 * kernel sin function on [-pi/4, pi/4], pi/4 ~ 0.7854
 * Input x is assumed to be bounded by ~pi/4 in magnitude.
 * Input y is the tail of x.
 * Input iy indicates whether y is 0. (if iy=0, y assume to be 0).
 */
fn kernel_sin(x: f64, y: f64, iy: i32) -> f64 {
    const S1: f64 = -1.66666666666666324348e-01; /* 0xBFC55555, 0x55555549 */
    const S2: f64 = 8.33333333332248946124e-03; /* 0x3F811111, 0x1110F8A6 */
    const S3: f64 = -1.98412698298579493134e-04; /* 0xBF2A01A0, 0x19C161D5 */
    const S4: f64 = 2.75573137070700676789e-06; /* 0x3EC71DE3, 0x57B1FE7D */
    const S5: f64 = -2.50507602534068634195e-08; /* 0xBE5AE5E6, 0x8A2B9CEB */
    const S6: f64 = 1.58969099521155010221e-10; /* 0x3DE5D93A, 0x5ACFD57C */

    let ix = high_word(x) & 0x7FFFFFFF; /* high word of x */
    if ix < 0x3E400000 && x as i32 == 0 {
        /* |x| < 2**-27 */
        return x; /* generate inexact */
    }
    let z = x * x;
    let v = z * x;
    let r = mad(z, mad(z, mad(z, mad(z, S6, S5), S4), S3), S2);
    if iy == 0 {
        mad(v, mad(z, r, S1), x)
    } else {
        x - mad(-v, S1, mad(z, mad(HALF, y, -(v * r)), -y))
    }
}

pub(crate) fn sin(x: f64) -> f64 {
    let ix = high_word(x) & 0x7FFFFFFF;
    if ix <= 0x3FE921FB {
        /* |x| ~< pi/4 */
        return kernel_sin(x, 0.0, 0);
    }
    if ix >= 0x7FF00000 {
        /* sin(Inf or NaN) is NaN */
        return x - x;
    }
    /* argument reduction needed */
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => kernel_sin(y0, y1, 1),
        1 => kernel_cos(y0, y1),
        2 => -kernel_sin(y0, y1, 1),
        _ => -kernel_cos(y0, y1),
    }
}

pub(crate) fn cos(x: f64) -> f64 {
    let ix = high_word(x) & 0x7FFFFFFF;
    if ix <= 0x3FE921FB {
        /* |x| ~< pi/4 */
        return kernel_cos(x, 0.0);
    }
    if ix >= 0x7FF00000 {
        /* cos(Inf or NaN) is NaN */
        return x - x;
    }
    /* argument reduction needed */
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => kernel_cos(y0, y1),
        1 => -kernel_sin(y0, y1, 1),
        2 => -kernel_cos(y0, y1),
        _ => kernel_sin(y0, y1, 1),
    }
}
