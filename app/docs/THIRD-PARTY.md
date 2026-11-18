# 외부 코드·라이선스 기록

Cargo.lock의 레지스트리 버전·체크섬으로 고정한다. 레지스트리 배포본에 참조 커밋이 없는 경우 커밋을 추정하지 않는다. 레지스트리 패키지는 수정하지 않았다. Tolaria 및 oh-my-pi 코드를 런타임으로 탑재하지 않는다. 추가 구문 정의의 고정 출처·라이선스와 VB 문법 변환·자체 MDX 어댑터는 [구문 정의 기록](third-party/markdown-syntax-provenance.md)에 기록한다.

## 내장 글꼴

SUIT v2.0.5 (Regular, Medium, SemiBold, Bold OTF)를 수정 없이 앱에 포함한다. SIL Open Font License 1.1이며 원문은 앱의 `Contents/Resources/Licenses/SUIT-OFL.txt`와 저장소의 [OFL.txt](../assets/fonts/OFL.txt)에 있다. 출처와 고정 커밋은 [폰트 기록](../assets/fonts/README.md)에 명시한다.

## Cargo 의존성
## 채팅 차트

Mermaid 11.12.2 (MIT)를 내장한다. 고정 출처·체크섬은 [런타임 기록](third-party/mermaid-runtime.md), 라이선스 원문은 [LICENSE](third-party/mermaid-runtime-LICENSE)에 있다.


| 패키지 | 버전 | 라이선스 | 참조 커밋 | 출처 | 수정 |
|---|---|---|---|---|---|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | 89a031a0f42eeff31c70dc598b398cbf31f1680f | https://github.com/oyvindln/adler2 | 없음 |
| android_system_properties | 0.1.6 | MIT OR Apache-2.0 | f859a346ba256d92ef047e7a49d6a2f59972257b | https://github.com/nical/android_system_properties | 없음 |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | 1dbe1862aae650423e3361fbd20b7d17c5109cc3 | https://github.com/dtolnay/anyhow | 없음 |
| arbitrary | 1.5.0 | MIT OR Apache-2.0 | 9807b599146c301a67e20692635352c9db70bff2 | https://github.com/rust-fuzz/arbitrary/ | 없음 |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | 2799b09c24e6632f8e653c5cd8fc303e85a906ba | https://github.com/cuviper/autocfg | 없음 |
| bincode | 1.3.3 | MIT | c44b5e364e7084cdbabf9f94b63a3c7f32b8fb68 | https://github.com/servo/bincode | 없음 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | 80ce9b545acb0bd42150695fc351889cac1d8eb4 | https://github.com/bitflags/bitflags | 없음 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | 6d35952d3d3b124bc1049ad6fb406b42b1ce4bfe | https://github.com/RustCrypto/utils | 없음 |
| block2 | 0.6.2 | MIT | b4167b582b2f75f9a1be75495c41b765344fd03c | https://github.com/madsmtm/objc2 | 없음 |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | 84654ace6be4444da3ff102a0a0af3b38c4df4fb | https://github.com/fitzgen/bumpalo | 없음 |
| bumpalo | 3.20.4 | MIT OR Apache-2.0 | 9b301879f9c53ea2403d474187668fc1a46f3a1c | https://github.com/fitzgen/bumpalo | 없음 |
| cc | 1.11.2 | MIT OR Apache-2.0 | f053f4ca6be6bdf37b7c4ae657981e0ec2fba879 | https://github.com/rust-lang/cc-rs | 없음 |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | 8b0a2b0fc1fa3b3741b9e72895f390cf499fe36f | https://github.com/rust-lang/cfg-if | 없음 |
| chrono | 0.4.45 | MIT OR Apache-2.0 | 170338250e836976a211e64728ec956e45e78a39 | https://github.com/chronotope/chrono | 없음 |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | 652bab0a62d87df8a974ffad2ad9f7be218a8b56 | https://github.com/servo/core-foundation-rs | 없음 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | 9d92d5e95ab4c07c5d8bfd024bf2a17e96d20feb | https://github.com/RustCrypto/utils | 없음 |
| crc32fast | 1.5.5 | MIT OR Apache-2.0 | 8a93d2a70c00ab3e72beda42014b18fad8e496ea | https://github.com/srijs/rust-crc32fast | 없음 |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | 2920c984290229ab4e0ca0452ef09e48a82063f3 | https://github.com/crossbeam-rs/crossbeam | 없음 |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | 66a997ccf5776ce873da5a48e2885a9908e940f6 | https://github.com/RustCrypto/traits | 없음 |
| derive_arbitrary | 1.5.0 | MIT OR Apache-2.0 | 9807b599146c301a67e20692635352c9db70bff2 | https://github.com/rust-fuzz/arbitrary | 없음 |
| digest | 0.10.7 | MIT OR Apache-2.0 | 344389411fd9718a0742435152e933a9e71461ee | https://github.com/RustCrypto/traits | 없음 |
| dispatch2 | 0.3.1 | Zlib OR Apache-2.0 OR MIT | 8852b424193ca41602281b3d7540d7c8ed51e49a | https://github.com/madsmtm/objc2 | 없음 |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | 4ee88130bd4ab3229e785533da2af641be22deac | https://github.com/indexmap-rs/equivalent | 없음 |
| errno | 0.3.15 | MIT OR Apache-2.0 | 4e3a9fc63211c0a958036dce5f9dbbeae54f8bc9 | https://github.com/lambda-fairy/rust-errno | 없음 |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 | 11cfa0558045e52bdb473c9eab6a1eb35a54e302 | https://github.com/sfackler/rust-fallible-iterator | 없음 |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 | 레지스트리 배포본 · 커밋 미기재 | https://github.com/sfackler/fallible-streaming-iterator | 없음 |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | 7a1cc2c69ebde6cb3442809213db3fe19a2cb0ba | https://github.com/smol-rs/fastrand | 없음 |
| find-msvc-tools | 0.1.15 | MIT OR Apache-2.0 | ffbb10da608f715ed69a4561c4cbdca7c6fbbba6 | https://github.com/rust-lang/cc-rs | 없음 |
| flate2 | 1.1.11 | MIT OR Apache-2.0 | cd06529164c5447958f309e649ffe6c9d6329865 | https://github.com/rust-lang/flate2-rs | 없음 |
| fnv | 1.0.7 | Apache-2.0 / MIT | 4b4784ebfd3332dc61f0640764d6f1140e03a9ab | https://github.com/servo/rust-fnv | 없음 |
| foldhash | 0.2.0 | Zlib | 8f878c636fda9c9e93384824ea45e06d03f009f5 | https://github.com/orlp/foldhash | 없음 |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | 705e6b5c0f06535b1aac1cb1989a172b3d45be8c | https://github.com/rust-lang/futures-rs | 없음 |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | 705e6b5c0f06535b1aac1cb1989a172b3d45be8c | https://github.com/rust-lang/futures-rs | 없음 |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | 705e6b5c0f06535b1aac1cb1989a172b3d45be8c | https://github.com/rust-lang/futures-rs | 없음 |
| generic-array | 0.14.7 | MIT | 1397d2bb40c406901b7827161b73b34b056cbe3a | https://github.com/fizyk20/generic-array.git | 없음 |
| getopts | 0.2.24 | MIT OR Apache-2.0 | 40846923f0ce024bcf8d3e9100cab64a38c8dd08 | https://github.com/rust-lang/getopts | 없음 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | 5e7cd5733536844a9856dc7259bd4696bbe5e3ae | https://github.com/rust-random/getrandom | 없음 |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 | 1876e4f02708b93903d55ef598f68e82a826518f | https://github.com/rust-lang/hashbrown | 없음 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | c62a63a61b7caf2de8f9ecb7b06a66b0ab6bdf3d | https://github.com/rust-lang/hashbrown | 없음 |
| hashlink | 0.11.1 | MIT OR Apache-2.0 | fe151cc410a22009eb56157ce9ec101daad174ae | https://github.com/djc/hashlink | 없음 |
| hex | 0.4.3 | MIT OR Apache-2.0 | b2b4370b5bf021b98ee7adc92233e8de3f2de792 | https://github.com/KokaKiwi/rust-hex | 없음 |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 | 3726968181ae67d95c8de2f750a95b9c703ae4c8 | https://github.com/strawlab/iana-time-zone | 없음 |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 | 668ef37a89162b7d24fb1cf8ee90054a8a2f3ad9 | https://github.com/strawlab/iana-time-zone | 없음 |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | 41a870887c4c77adf665886e63df08f406bfe37a | https://github.com/indexmap-rs/indexmap | 없음 |
| itoa | 1.0.19 | MIT OR Apache-2.0 | 9339ca99a43c550b5ee36e2afe524d1c90413461 | https://github.com/dtolnay/itoa | 없음 |
| js-sys | 0.3.107 | MIT OR Apache-2.0 | c93b8ae848d4d9e6beaeaf9516e18a7cadd818a2 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys | 없음 |
| libc | 0.2.191 | MIT OR Apache-2.0 | ccc2ff4aaab962f7fde7b8edda7379c045b1ca12 | https://github.com/rust-lang/libc | 없음 |
| libsqlite3-sys | 0.36.0 | MIT | 35b3be2436a63d21701d1d110661e6392831fea0 | https://github.com/rusqlite/rusqlite | 없음 |
| linked-hash-map | 0.5.6 | MIT/Apache-2.0 | dfc4a57278b79314b0c1892d6c3918f2e00e249b | https://github.com/contain-rs/linked-hash-map | 없음 |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 0e2918cf3e366d9c923d4ca05f169b49d826db56 | https://github.com/sunfishcode/linux-raw-sys | 없음 |
| log | 0.4.35 | MIT OR Apache-2.0 | a46450ba763280c29d35b53aa8ad0ff1fc1d78d0 | https://github.com/rust-lang/log | 없음 |
| memchr | 2.8.3 | Unlicense OR MIT | 5fdb40c054e1fff359a2f7bdf7f87a13b34b465d | https://github.com/BurntSushi/memchr | 없음 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | 4e582392df3a739d2b0dfd2c537dc33e8942be38 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide | 없음 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | 7ec3d41d39b28190ec1d42db38021107b3951f3a | https://github.com/rust-num/num-traits | 없음 |
| objc2 | 0.6.5 | MIT | d7d2fa23ceaa5e6096c923b081040e5d81b3b9df | https://github.com/madsmtm/objc2 | 없음 |
| objc2-app-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-cloud-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-core-data | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-core-graphics | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-core-image | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-core-text | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-core-video | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-encode | 4.1.0 | MIT | 8d214f5477365ffcbcbb7de058c86ed9a518efb7 | https://github.com/madsmtm/objc2 | 없음 |
| objc2-foundation | 0.3.2 | MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-io-surface | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| objc2-quartz-core | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 7b1abfd750a2cacaea71d6a56ecfb83cb7de560b | https://github.com/madsmtm/objc2 | 없음 |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | 80fe900b21f6d76c1a2ed74d3343e8a3a88c46d0 | https://github.com/matklad/once_cell | 없음 |
| onig | 6.5.3 | MIT | 3572dd210e9bbfbf056c12f3a65bfcc9fede7866 | https://github.com/iwillspeak/rust-onig | 없음 |
| onig_sys | 69.9.3 | MIT | 3572dd210e9bbfbf056c12f3a65bfcc9fede7866 | https://github.com/rust-onig/rust-onig | 없음 |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | 3bdf763446aa78f90e3bdac1ef583e014832ab4c | https://github.com/taiki-e/pin-project-lite | 없음 |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | a8852cc148f8f7d504bf3a62444c298dcd3637f0 | https://github.com/rust-lang/pkg-config-rs | 없음 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | ed8a5497669cd63db33bf24646f261b012bbbc4a | https://github.com/dtolnay/proc-macro2 | 없음 |
| pulldown-cmark | 0.13.6 | MIT | 014b61d38d235f382935bb676bab872b7680afa2 | https://github.com/raphlinus/pulldown-cmark | 없음 |
| pulldown-cmark-escape | 0.11.0 | MIT | d7632acbe066b81a7f4b23c72fdb18c85f9cb1ea | https://github.com/raphlinus/pulldown-cmark | 없음 |
| quick-xml | 0.37.5 | MIT | a018365abc4743b4abbd53892a7362b3f239bbd1 | https://github.com/tafia/quick-xml | 없음 |
| quote | 1.0.47 | MIT OR Apache-2.0 | 723dcb47d3f0ddc896e17287c8a8d3f2ea2317d5 | https://github.com/dtolnay/quote | 없음 |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | 7e1b0322d31d625f81a5656096330934f9cd835d | https://github.com/r-efi/r-efi | 없음 |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | 140167995737fa11dfe11b8af8b9aa143b790b4e | https://github.com/rust-lang/regex | 없음 |
| rsqlite-vfs | 0.1.1 | MIT | 1bb309784f25c401ddf95131dcde4c4cdad7aee7 | registry+https://github.com/rust-lang/crates.io-index | 없음 |
| rusqlite | 0.38.0 | MIT | 35b3be2436a63d21701d1d110661e6392831fea0 | https://github.com/rusqlite/rusqlite | 없음 |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 4a563ad698b2c20ff848515821023d2265a788d0 | https://github.com/bytecodealliance/rustix | 없음 |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | 3a7c76605450b9a7299c6502a421909de9126a59 | https://github.com/dtolnay/rustversion | 없음 |
| same-file | 1.0.6 | Unlicense/MIT | 5799cd323b8eefd17a089c950dac113f66c89c9e | https://github.com/BurntSushi/same-file | 없음 |
| serde | 1.0.229 | MIT OR Apache-2.0 | 7fc3b4c30c94f73a96ebd1553f2b090d928fc3a8 | https://github.com/serde-rs/serde | 없음 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | 7fc3b4c30c94f73a96ebd1553f2b090d928fc3a8 | https://github.com/serde-rs/serde | 없음 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | 7fc3b4c30c94f73a96ebd1553f2b090d928fc3a8 | https://github.com/serde-rs/serde | 없음 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | 20572a6d066958542d79cdd6cf1a7cd9f0c8e1f8 | https://github.com/serde-rs/json | 없음 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | 82c36a428f8d6f05f3bfccdedb243e9d1f85359d | https://github.com/RustCrypto/hashes | 없음 |
| shlex | 2.0.1 | MIT OR Apache-2.0 | e82b1411beb7c92871c2c078c9ab415bbcf207ef | https://github.com/comex/rust-shlex | 없음 |
| simd-adler32 | 0.3.10 | MIT | d93164f98d7fe2daf883f2e1bec82778dd26199a | https://github.com/mcountryman/simd-adler32 | 없음 |
| slab | 0.4.12 | MIT | a1e4346070a48c936d808de75191dee5d01e433c | https://github.com/tokio-rs/slab | 없음 |
| smallvec | 1.16.5 | MIT OR Apache-2.0 | 6e900059c51b7c0f5580e13e9e215a50b6be9d27 | https://github.com/servo/rust-smallvec | 없음 |
| sqlite-wasm-rs | 0.5.5 | MIT | f2bb5109c07bf3d7ab800704c0f2373e6c7aa3b4 | https://github.com/Spxg/sqlite-wasm-rs | 없음 |
| syn | 2.0.119 | MIT OR Apache-2.0 | 3295f9e9841785ac88a5e558c884854d5fb7d67f | https://github.com/dtolnay/syn | 없음 |
| syn | 3.0.10 | MIT OR Apache-2.0 | 97ec37dda8a893a460066da694fd1eb7cb30538a | https://github.com/dtolnay/syn | 없음 |
| syntect | 5.3.0 | MIT | e4670846ecf16d8832db6c43d531bec466214e27 | https://github.com/trishume/syntect | 없음 |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | 5c8fa12eb584931b4f1bccfde87eb72fbfa7dc61 | https://github.com/Stebalien/tempfile | 없음 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | b1827ee06f81a7d7f954e676771e19a97f9f8e3b | https://github.com/dtolnay/thiserror | 없음 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | b1827ee06f81a7d7f954e676771e19a97f9f8e3b | https://github.com/dtolnay/thiserror | 없음 |
| two-face | 0.5.2+bat-0.26.1 | MIT OR Apache-2.0 | 67b64e632ceefe70e5f20f3eb797d9e33560a11c | https://codeberg.org/CosmicHarper/two-face | 없음 |
| typenum | 1.20.1 | MIT OR Apache-2.0 | 0db9a0f731981f29266b63586c29fa07e4477b1a | https://github.com/paholg/typenum | 없음 |
| unicase | 2.10.0 | MIT OR Apache-2.0 | 1e739ff80fe2297c52efda9d05f0bea1038a7eb9 | https://github.com/seanmonstar/unicase | 없음 |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | c1ac3afb721f3cea6527bc2255340a04d0de3ca0 | https://github.com/dtolnay/unicode-ident | 없음 |
| unicode-width | 0.2.3 | MIT OR Apache-2.0 | f8c8b20d0ef0102dc4c136bcd5a1b39d2aef8a69 | https://github.com/unicode-rs/unicode-width | 없음 |
| uuid | 1.29.2 | Apache-2.0 OR MIT | 5205a6ea18425e6781f2ac09e896d88153d9cd2d | https://github.com/uuid-rs/uuid | 없음 |
| vcpkg | 0.2.15 | MIT/Apache-2.0 | e7d37e095ef2aa85c80242aa3850d442832b7b54 | https://github.com/mcgoo/vcpkg-rs | 없음 |
| version_check | 0.9.5 | MIT/Apache-2.0 | d77ef9f27cc336719b2d839d09ee6635dd22f758 | https://github.com/SergioBenitez/version_check | 없음 |
| walkdir | 2.5.0 | Unlicense/MIT | 4f26be4d450910916ea11533b2efc52b9a6483bc | https://github.com/BurntSushi/walkdir | 없음 |
| wasm-bindgen | 0.2.130 | MIT OR Apache-2.0 | c93b8ae848d4d9e6beaeaf9516e18a7cadd818a2 | https://github.com/wasm-bindgen/wasm-bindgen | 없음 |
| wasm-bindgen-macro | 0.2.130 | MIT OR Apache-2.0 | c93b8ae848d4d9e6beaeaf9516e18a7cadd818a2 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro | 없음 |
| wasm-bindgen-macro-support | 0.2.130 | MIT OR Apache-2.0 | c93b8ae848d4d9e6beaeaf9516e18a7cadd818a2 | https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support | 없음 |
| wasm-bindgen-shared | 0.2.130 | MIT OR Apache-2.0 | c93b8ae848d4d9e6beaeaf9516e18a7cadd818a2 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared | 없음 |
| winapi-util | 0.1.11 | Unlicense OR MIT | 803874c57dc1f10ecd42f7c86d9de72f53818432 | https://github.com/BurntSushi/winapi-util | 없음 |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | 32c3144490c016fe496a0aed769bce60987a2e9d | https://github.com/microsoft/windows-rs | 없음 |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | 32c3144490c016fe496a0aed769bce60987a2e9d | https://github.com/microsoft/windows-rs | 없음 |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | 32c3144490c016fe496a0aed769bce60987a2e9d | https://github.com/microsoft/windows-rs | 없음 |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | d468916ac27a36fb8a12bafc1bf5c0ec2fe92238 | https://github.com/microsoft/windows-rs | 없음 |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | 32c3144490c016fe496a0aed769bce60987a2e9d | https://github.com/microsoft/windows-rs | 없음 |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | 32c3144490c016fe496a0aed769bce60987a2e9d | https://github.com/microsoft/windows-rs | 없음 |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | 32c3144490c016fe496a0aed769bce60987a2e9d | https://github.com/microsoft/windows-rs | 없음 |
| yaml-rust | 0.4.5 | MIT/Apache-2.0 | 6cd3ce4abe6894443645c48bdc375808ec911493 | https://github.com/chyh1990/yaml-rust | 없음 |
| zip | 2.6.1 | MIT | 1d42731efcad72e9f653a5ca46ed8306f4a3ca72 | https://github.com/zip-rs/zip2.git | 없음 |
| zlib-rs | 0.6.8 | Zlib | 0254e07204884bfc902d6ebb68c8b703d0ea49cf | https://github.com/trifectatechfoundation/zlib-rs | 없음 |
| zmij | 1.0.23 | MIT | 7b7cc48b58028e8af7be87e94c0c1c8936f1a57c | https://github.com/dtolnay/zmij | 없음 |
| zopfli | 0.8.4 | Apache-2.0 | 62df3aebea9f458558984b106f87e980d4077c2f | https://github.com/zopfli-rs/zopfli | 없음 |
