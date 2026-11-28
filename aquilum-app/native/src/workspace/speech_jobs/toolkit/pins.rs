pub(super) struct Pin {
    pub(super) name: &'static str,
    pub(super) bytes: u64,
    pub(super) sha256: &'static str,
}

const fn pin(name: &'static str, bytes: u64, sha256: &'static str) -> Pin {
    Pin {
        name,
        bytes,
        sha256,
    }
}

pub(super) const FFMPEG_FILES: [Pin; 2] = [
    pin(
        "ffprobe.exe",
        110_374_912,
        "df0199f606faa88e9ad192732a7c3987acfb647663534c297772e17730611a08",
    ),
    pin(
        "ffmpeg.exe",
        110_590_976,
        "ed8f66cb877087a96f5bbb40a2944f0e764f2ad8870d5cb647360d912e62bc07",
    ),
];

pub(super) const WHISPER_FILES: [Pin; 13] = [
    pin(
        "whisper-cli.exe",
        479_744,
        "331dba46d6427105d2b802cdbc7eae916ea1c5abf9b0d3b5cfe460d8db8e4366",
    ),
    pin(
        "whisper.dll",
        1_370_112,
        "034396d75bb5720c1851674ff64a96192b58f6927b68d334ebd254ce6cad9674",
    ),
    pin(
        "ggml.dll",
        62_464,
        "31fe18616d872af9b6134b47a6dff8072ae4ecbe5df0273f4f8dc36766c9db8e",
    ),
    pin(
        "ggml-base.dll",
        693_248,
        "bede0d6b2d387236ecd2b5dd676d51c8984b5b6e0ae461a12174e9c01345c5f0",
    ),
    pin(
        "ggml-cpu-x64.dll",
        866_816,
        "4b4b05caba40d4309b5d710ef74c59ba190516b8489f27bdb1ba11ae33c6e9d3",
    ),
    pin(
        "ggml-cpu-sse42.dll",
        862_720,
        "f060318772c4c2dbac312bbfff6ea700b85dbc502bb38585a7186d06d549301a",
    ),
    pin(
        "ggml-cpu-sandybridge.dll",
        1_026_560,
        "04edbdebc01a1d421062a960881a4f34d520d99693999518243f332406444c6e",
    ),
    pin(
        "ggml-cpu-haswell.dll",
        1_010_688,
        "1306b1014c8118d27234d8ddc0e66ced1f8c8476bf85071595fd182ea9b7263b",
    ),
    pin(
        "ggml-cpu-cannonlake.dll",
        1_049_600,
        "74127b40abd4f595d9db6af0fa313a02e18ff42fc228bfa05281e7c788dd4507",
    ),
    pin(
        "ggml-cpu-alderlake.dll",
        1_009_664,
        "d8f5c56ea490394eeae8b271650f2647aecd039a542d6df9e0140726e62a4603",
    ),
    pin(
        "ggml-cpu-skylakex.dll",
        1_049_600,
        "1774714cf93621406ff35344b54715619a83242ae3e331e44d98a3b35ce801be",
    ),
    pin(
        "ggml-cpu-cascadelake.dll",
        1_061_376,
        "4b11c427b844eeac87529af9a8d94b35aad2024666beb2a35e59fe3077e4b81e",
    ),
    pin(
        "ggml-cpu-icelake.dll",
        1_061_376,
        "2b4dc4c66e7d8fe2807283816e9cf3b7f7d0556f59df34069871e37eab8aed40",
    ),
];

pub(super) const MODEL: Pin = pin(
    "ggml-tiny.bin",
    77_691_713,
    "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
);

pub(super) const VAD: Pin = pin(
    "ggml-silero-v6.2.0.bin",
    885_098,
    "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
);
