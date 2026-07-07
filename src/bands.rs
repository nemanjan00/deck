//! Static band plan (region-1-flavored, config-extensible) for the scope
//! overlay. Coarse on purpose — orientation, not regulation.

pub struct Band {
    pub from: u64,
    pub to: u64,
    pub label: &'static str,
}

pub const BANDS: &[Band] = &[
    Band {
        from: 135_700,
        to: 137_800,
        label: "2200m",
    },
    Band {
        from: 1_810_000,
        to: 2_000_000,
        label: "160m",
    },
    Band {
        from: 3_500_000,
        to: 3_800_000,
        label: "80m",
    },
    Band {
        from: 5_351_500,
        to: 5_366_500,
        label: "60m",
    },
    Band {
        from: 7_000_000,
        to: 7_200_000,
        label: "40m",
    },
    Band {
        from: 10_100_000,
        to: 10_150_000,
        label: "30m",
    },
    Band {
        from: 14_000_000,
        to: 14_350_000,
        label: "20m",
    },
    Band {
        from: 18_068_000,
        to: 18_168_000,
        label: "17m",
    },
    Band {
        from: 21_000_000,
        to: 21_450_000,
        label: "15m",
    },
    Band {
        from: 24_890_000,
        to: 24_990_000,
        label: "12m",
    },
    Band {
        from: 26_965_000,
        to: 27_405_000,
        label: "CB",
    },
    Band {
        from: 28_000_000,
        to: 29_700_000,
        label: "10m",
    },
    Band {
        from: 50_000_000,
        to: 52_000_000,
        label: "6m",
    },
    Band {
        from: 87_500_000,
        to: 108_000_000,
        label: "FM bcast",
    },
    Band {
        from: 108_000_000,
        to: 118_000_000,
        label: "VOR/ILS",
    },
    Band {
        from: 118_000_000,
        to: 137_000_000,
        label: "airband",
    },
    Band {
        from: 144_000_000,
        to: 146_000_000,
        label: "2m",
    },
    Band {
        from: 156_000_000,
        to: 162_050_000,
        label: "marine",
    },
    Band {
        from: 161_975_000,
        to: 162_025_000,
        label: "AIS",
    },
    Band {
        from: 174_000_000,
        to: 230_000_000,
        label: "DAB",
    },
    Band {
        from: 430_000_000,
        to: 440_000_000,
        label: "70cm",
    },
    Band {
        from: 433_050_000,
        to: 434_790_000,
        label: "ISM433",
    },
    Band {
        from: 446_000_000,
        to: 446_200_000,
        label: "PMR446",
    },
    Band {
        from: 863_000_000,
        to: 870_000_000,
        label: "ISM868",
    },
    Band {
        from: 1_090_000_000,
        to: 1_090_000_001,
        label: "ADS-B",
    },
    Band {
        from: 1_240_000_000,
        to: 1_300_000_000,
        label: "23cm",
    },
];

/// Bands overlapping [from, to].
pub fn in_range(from: u64, to: u64) -> impl Iterator<Item = &'static Band> {
    BANDS.iter().filter(move |b| b.to > from && b.from < to)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lookup() {
        let hits: Vec<_> = super::in_range(433_000_000, 447_000_000)
            .map(|b| b.label)
            .collect();
        assert!(hits.contains(&"70cm"));
        assert!(hits.contains(&"ISM433"));
        assert!(hits.contains(&"PMR446"));
        assert!(!hits.contains(&"2m"));
    }
}
