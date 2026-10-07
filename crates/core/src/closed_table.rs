#[macro_export]
macro_rules! closed_table {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident : $repr:ident {
            $($(#[$variant_meta:meta])* $variant:ident = $tag:literal => $spelling:literal),+ $(,)?
        }
        $(retired: [$($retired:literal),* $(,)?];)?
    ) => {
        $crate::closed_table!(@tagged
            [$(#[$meta])*] [$vis] $name $repr
            [$([$(#[$variant_meta])*] $variant = $tag)+]
            [$($($retired)*)?]
        );
        $crate::closed_table!(@spelled $name [$($variant => $spelling)+]);
    };
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident : $repr:ident {
            $($(#[$variant_meta:meta])* $variant:ident = $tag:literal),+ $(,)?
        }
        $(retired: [$($retired:literal),* $(,)?];)?
    ) => {
        $crate::closed_table!(@tagged
            [$(#[$meta])*] [$vis] $name $repr
            [$([$(#[$variant_meta])*] $variant = $tag)+]
            [$($($retired)*)?]
        );
    };
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident => $spelling:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
        $vis enum $name {
            $($(#[$variant_meta])* $variant),+
        }
        $crate::closed_table!(@all $name [$($variant)+]);
        $crate::closed_table!(@spelled $name [$($variant => $spelling)+]);
    };
    (@tagged
        [$($meta:tt)*] [$($vis:tt)*] $name:ident $repr:ident
        [$([$($variant_meta:tt)*] $variant:ident = $tag:literal)+]
        [$($retired:literal)*]
    ) => {
        $($meta)*
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
        #[repr($repr)]
        $($vis)* enum $name {
            $($($variant_meta)* $variant = $tag),+
        }
        $crate::closed_table!(@all $name [$($variant)+]);
        #[allow(dead_code)]
        impl $name {
            pub const RETIRED: [$repr; <[$repr]>::len(&[$($retired),*])] = [$($retired),*];

            #[must_use]
            pub const fn tag(self) -> $repr {
                self as $repr
            }

            #[must_use]
            pub const fn from_tag(tag: $repr) -> ::core::option::Option<Self> {
                match tag {
                    $($tag => ::core::option::Option::Some(Self::$variant),)+
                    _ => ::core::option::Option::None,
                }
            }
        }
        const _: () = {
            #[deny(unreachable_patterns)]
            let _ = |tag: $repr| match tag {
                $($tag => (),)+
                $($retired => (),)*
                _ => (),
            };
        };
    };
    (@all $name:ident [$($variant:ident)+]) => {
        #[allow(dead_code)]
        impl $name {
            pub const COUNT: usize = [$(Self::$variant),+].len();

            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),+];
        }
    };
    (@spelled $name:ident [$($variant:ident => $spelling:literal)+]) => {
        #[allow(dead_code)]
        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling),+
                }
            }

            #[must_use]
            #[allow(clippy::should_implement_trait)]
            pub fn from_str(spelling: &str) -> ::core::option::Option<Self> {
                match spelling {
                    $($spelling => ::core::option::Option::Some(Self::$variant),)+
                    _ => ::core::option::Option::None,
                }
            }
        }
        impl ::core::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
        const _: () = {
            #[deny(unreachable_patterns)]
            let _ = |spelling: &str| match spelling {
                $($spelling => (),)+
                _ => (),
            };
        };
    };
}

#[cfg(test)]
mod tests {
    crate::closed_table! {
        enum Spelled {
            First => "first",
            Second => "second",
            Third => "third",
        }
    }

    crate::closed_table! {
        enum Numbered: u16 {
            Nine = 9,
            Two = 2,
            Seven = 7,
        }
        retired: [3, 4];
    }

    crate::closed_table! {
        enum Both: u32 {
            Low = 1 => "low",
            High = 40 => "high",
        }
    }

    #[test]
    fn spelled_table_round_trips_every_arm_and_refuses_unknown_spellings() {
        assert_eq!(Spelled::COUNT, 3);
        assert_eq!(
            Spelled::ALL,
            [Spelled::First, Spelled::Second, Spelled::Third]
        );
        assert_eq!(
            Spelled::ALL.map(Spelled::as_str),
            ["first", "second", "third"]
        );
        for arm in Spelled::ALL {
            assert_eq!(Spelled::from_str(arm.as_str()), Some(arm));
            assert_eq!(arm.to_string(), arm.as_str());
        }
        assert_eq!(Spelled::from_str("fourth"), None);
        assert_eq!(Spelled::from_str("First"), None);
        assert_eq!(Spelled::from_str(""), None);
    }

    #[test]
    fn numbered_table_keeps_explicit_numbers_and_refuses_retired_ones() {
        assert_eq!(Numbered::COUNT, 3);
        assert_eq!(
            Numbered::ALL,
            [Numbered::Nine, Numbered::Two, Numbered::Seven]
        );
        assert_eq!(Numbered::ALL.map(Numbered::tag), [9, 2, 7]);
        assert_eq!(Numbered::RETIRED, [3, 4]);
        for arm in Numbered::ALL {
            assert_eq!(Numbered::from_tag(arm.tag()), Some(arm));
        }
        for number in [0, 1, 3, 4, 5, 6, 8, 10, u16::MAX] {
            assert_eq!(Numbered::from_tag(number), None, "{number}");
        }
    }

    #[test]
    fn table_with_numbers_and_spellings_derives_both_directions() {
        assert_eq!(Both::COUNT, 2);
        assert_eq!(Both::RETIRED, [] as [u32; 0]);
        assert_eq!(Both::Low.tag(), 1);
        assert_eq!(Both::High.tag(), 40);
        assert_eq!(Both::from_tag(40), Some(Both::High));
        assert_eq!(Both::from_str("low"), Some(Both::Low));
        assert_eq!(Both::from_tag(2), None);
        assert_eq!(Both::from_str("middle"), None);
        assert_eq!(format!("{}", Both::High), "high");
    }
}
