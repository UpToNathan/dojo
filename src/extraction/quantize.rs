use super::color::{Lab, Rgb};

struct Bucket {
    colors: Vec<Lab>,
    ranges: [(f64, f64); 3],
}

fn axes(color: Lab) -> [f64; 3] {
    [color.l, color.a, color.b]
}

impl Bucket {
    fn new(colors: Vec<Lab>) -> Self {
        let mut ranges = [(f64::INFINITY, f64::NEG_INFINITY); 3];
        for color in &colors {
            for (range, value) in ranges.iter_mut().zip(axes(*color)) {
                range.0 = range.0.min(value);
                range.1 = range.1.max(value);
            }
        }
        Self { colors, ranges }
    }

    fn volume(&self) -> f64 {
        self.ranges
            .iter()
            .map(|(min, max)| max - min)
            .product::<f64>()
            * self.colors.len() as f64
    }

    fn split(mut self) -> (Self, Self) {
        let [l, a, b] = self.ranges.map(|(min, max)| max - min);
        let axis = if l * 1.2 >= a && l * 1.2 >= b {
            0
        } else if a >= b {
            1
        } else {
            2
        };
        self.colors
            .sort_by(|a, b| axes(*a)[axis].total_cmp(&axes(*b)[axis]));
        let right = self.colors.split_off(self.colors.len() / 2);
        (Self::new(self.colors), Self::new(right))
    }

    fn average(&self) -> Rgb {
        let mut sum = Lab::default();
        for color in &self.colors {
            sum.l += color.l;
            sum.a += color.a;
            sum.b += color.b;
        }
        let count = self.colors.len() as f64;
        Lab {
            l: sum.l / count,
            a: sum.a / count,
            b: sum.b / count,
        }
        .rgb()
    }
}

pub(super) fn dominant(pixels: &[Rgb], target: usize) -> (Vec<Rgb>, Vec<usize>) {
    let mut boosted = Vec::with_capacity(pixels.len() * 2);
    for pixel in pixels {
        let lab = pixel.lab();
        boosted.push(lab);
        let chroma = lab.lch().c;
        if chroma > 0.04 {
            let extra = (((chroma - 0.04) / 0.16 * 3.).round() as usize).clamp(1, 3);
            boosted.extend(std::iter::repeat_n(lab, extra));
        }
    }
    let mut buckets = vec![Bucket::new(boosted)];
    while buckets.len() < target {
        let mut best = None;
        let mut largest = 0.;
        for (index, bucket) in buckets.iter().enumerate() {
            if bucket.colors.len() > 1 && bucket.volume() > largest {
                largest = bucket.volume();
                best = Some(index);
            }
        }
        let Some(index) = best else { break };
        let (left, right) = buckets.remove(index).split();
        buckets.insert(index, right);
        buckets.insert(index, left);
    }
    let mut result: Vec<_> = buckets
        .iter()
        .map(|b| (b.average(), b.colors.len()))
        .collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result.into_iter().unzip()
}
