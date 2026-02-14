# Third-Party Licenses

This file lists all third-party dependencies used by BetCode and their licenses.
Generated from Cargo.lock. When distributing BetCode binaries, this file must
be included alongside the binary.

## Summary

| License | Crates |
|---------|--------|
| Apache-2.0 | 294 |
| Apache-2.0 WITH LLVM-exception | 6 |
| BSD-1-Clause | 1 |
| BSD-2-Clause | 1 |
| BSD-3-Clause | 7 |
| BSL-1.0 | 1 |
| CC0-1.0 | 1 |
| CDLA-Permissive-2.0 | 2 |
| ISC | 9 |
| LGPL-2.1-or-later | 1 |
| MIT | 372 |
| MPL-2.0 | 2 |
| Unicode-3.0 | 19 |
| Unlicense | 5 |
| Zlib | 2 |

## Apache-2.0

Used by:
- aead@0.5.2
- allocator-api2@0.2.21
- anstream@0.6.21
- anstyle-parse@0.2.7
- anstyle-query@1.1.5
- anstyle-wincon@3.0.11
- anstyle@1.0.13
- anyhow@1.0.101
- argon2@0.5.3
- async-trait@0.1.89
- atomic-waker@1.1.2
- autocfg@1.5.0
- base16ct@0.2.0
- base64@0.22.1
- base64ct@1.8.3
- bitflags@1.3.2
- bitflags@2.10.0
- blake2@0.10.6
- block-buffer@0.10.4
- bumpalo@3.19.1
- cc@1.2.55
- cesu8@1.1.0
- cfg-if@1.0.4
- chacha20@0.10.0
- chacha20@0.9.1
- chacha20poly1305@0.10.1
- cipher@0.4.4
- clap@4.5.58
- clap_builder@4.5.58
- clap_derive@4.5.55
- clap_lex@1.0.0
- colorchoice@1.0.4
- concurrent-queue@2.5.0
- const-oid@0.9.6
- core-foundation-sys@0.8.7
- core-foundation@0.10.1
- core-foundation@0.9.4
- cpufeatures@0.2.17
- cpufeatures@0.3.0
- crc-catalog@2.4.0
- crc@3.4.0
- crossbeam-queue@0.3.12
- crossbeam-utils@0.8.21
- crypto-bigint@0.5.5
- crypto-common@0.1.7
- curve25519-dalek-derive@0.1.1
- der@0.7.10
- deranged@0.5.6
- digest@0.10.7
- dirs-sys@0.5.0
- dirs@6.0.0
- displaydoc@0.2.5
- document-features@0.2.12
- ecdsa@0.16.9
- ed25519@2.2.3
- either@1.15.0
- elliptic-curve@0.13.8
- encode_unicode@1.0.0
- encoding_rs@0.8.35
- equivalent@1.0.2
- errno@0.3.14
- event-listener@5.4.1
- fastrand@2.3.0
- ff@0.13.1
- fiat-crypto@0.2.9
- filetime@0.2.27
- find-msvc-tools@0.1.9
- fixedbitset@0.5.7
- flume@0.11.1
- fnv@1.0.7
- form_urlencoded@1.2.2
- futures-channel@0.3.31
- futures-core@0.3.31
- futures-executor@0.3.31
- futures-intrusive@0.5.0
- futures-io@0.3.31
- futures-macro@0.3.31
- futures-sink@0.3.31
- futures-task@0.3.31
- futures-util@0.3.31
- getrandom@0.2.17
- getrandom@0.3.4
- getrandom@0.4.1
- group@0.13.0
- hashbrown@0.15.5
- hashbrown@0.16.1
- hashlink@0.10.0
- heck@0.5.0
- hex@0.4.3
- hkdf@0.12.4
- hmac@0.12.1
- http@1.4.0
- httparse@1.10.1
- httpdate@1.0.3
- hyper-rustls@0.27.7
- hyper-timeout@0.5.2
- ident_case@1.0.1
- idna@1.1.0
- idna_adapter@1.2.1
- indexmap@2.13.0
- indoc@2.0.7
- inout@0.1.4
- ipnet@2.11.0
- iri-string@0.7.10
- is_terminal_polyfill@1.70.2
- itertools@0.14.0
- itoa@1.0.17
- jni-sys@0.3.0
- jni@0.21.1
- js-sys@0.3.85
- kasuari@0.4.11
- lazy_static@1.5.0
- libc@0.2.180
- line-clipping@0.3.5
- linux-raw-sys@0.11.0
- litrs@1.0.0
- lock_api@0.4.14
- log@0.4.29
- mime@0.3.17
- multimap@0.10.1
- notify-types@1.0.1
- num-bigint-dig@0.8.6
- num-bigint@0.4.6
- num-conv@0.2.0
- num-integer@0.1.46
- num-iter@0.1.45
- num-traits@0.2.19
- num_threads@0.1.7
- once_cell@1.21.3
- once_cell_polyfill@1.70.2
- opaque-debug@0.3.1
- openssl-probe@0.2.1
- p256@0.13.2
- p384@0.13.1
- parking@2.2.1
- parking_lot@0.12.5
- parking_lot_core@0.9.12
- password-hash@0.5.0
- pem-rfc7468@0.7.0
- percent-encoding@2.3.2
- petgraph@0.8.3
- pin-project-internal@1.1.10
- pin-project-lite@0.2.16
- pin-project@1.1.10
- pin-utils@0.1.0
- pkcs1@0.7.5
- pkcs8@0.10.2
- pkg-config@0.3.32
- poly1305@0.8.0
- powerfmt@0.2.0
- ppv-lite86@0.2.21
- prettyplease@0.2.37
- primeorder@0.13.6
- proc-macro2@1.0.106
- prost-build@0.14.3
- prost-derive@0.14.3
- prost-types@0.14.3
- prost@0.14.3
- pulldown-cmark-to-cmark@22.0.0
- quote@1.0.44
- r-efi@5.3.0
- rand@0.10.0
- rand@0.8.5
- rand_chacha@0.3.1
- rand_core@0.10.0
- rand_core@0.6.4
- rcgen@0.14.7
- regex-automata@0.4.14
- regex-syntax@0.8.9
- regex@1.12.3
- reqwest@0.13.2
- rfc6979@0.4.0
- ring@0.17.14
- rsa@0.9.10
- rustc_version@0.4.1
- rustix@1.1.3
- rustls-native-certs@0.8.3
- rustls-pki-types@1.14.0
- rustls-platform-verifier-android@0.1.1
- rustls-platform-verifier@0.6.2
- rustls@0.23.36
- rustversion@1.0.22
- ryu@1.0.23
- scopeguard@1.2.0
- sec1@0.7.3
- security-framework-sys@2.15.0
- security-framework@3.5.1
- semver@1.0.27
- serde@1.0.228
- serde_core@1.0.228
- serde_derive@1.0.228
- serde_json@1.0.149
- serde_path_to_error@0.1.20
- serde_spanned@0.6.9
- serde_urlencoded@0.7.1
- sha2@0.10.9
- shell-words@1.1.1
- shlex@1.3.0
- signal-hook-mio@0.2.5
- signal-hook-registry@1.4.8
- signal-hook@0.3.18
- signature@2.2.0
- smallvec@1.15.1
- socket2@0.6.2
- spki@0.7.3
- sqlx-core@0.8.6
- sqlx-macros-core@0.8.6
- sqlx-macros@0.8.6
- sqlx-sqlite@0.8.6
- sqlx@0.8.6
- stable_deref_trait@1.2.1
- static_assertions@1.1.0
- syn@2.0.114
- sync_wrapper@1.0.2
- system-configuration-sys@0.6.0
- system-configuration@0.7.0
- tempfile@3.25.0
- thiserror-impl@1.0.69
- thiserror-impl@2.0.18
- thiserror@1.0.69
- thiserror@2.0.18
- thread_local@1.1.9
- time-core@0.1.8
- time-macros@0.2.27
- time@0.3.47
- tokio-rustls@0.26.4
- toml@0.8.23
- toml_datetime@0.6.11
- toml_edit@0.22.27
- toml_write@0.1.2
- typenum@1.19.0
- unicase@2.9.0
- unicode-ident@1.0.23
- unicode-segmentation@1.12.0
- unicode-truncate@2.0.1
- unicode-width@0.2.2
- universal-hash@0.5.1
- url@2.5.8
- utf8_iter@1.0.4
- utf8parse@0.2.2
- uuid@1.20.0
- vcpkg@0.2.15
- version_check@0.9.5
- wasi@0.11.1+wasi-snapshot-preview1
- wasip2@1.0.2+wasi-0.2.9
- wasip3@0.4.0+wasi-0.3.0-rc-2026-01-06
- wasm-bindgen-futures@0.4.58
- wasm-bindgen-macro-support@0.2.108
- wasm-bindgen-macro@0.2.108
- wasm-bindgen-shared@0.2.108
- wasm-bindgen@0.2.108
- web-sys@0.3.85
- winapi-i686-pc-windows-gnu@0.4.0
- winapi-x86_64-pc-windows-gnu@0.4.0
- winapi@0.3.9
- windows-link@0.2.1
- windows-registry@0.6.1
- windows-result@0.4.1
- windows-strings@0.5.1
- windows-sys@0.45.0
- windows-sys@0.52.0
- windows-sys@0.60.2
- windows-sys@0.61.2
- windows-targets@0.42.2
- windows-targets@0.52.6
- windows-targets@0.53.5
- windows_aarch64_gnullvm@0.42.2
- windows_aarch64_gnullvm@0.52.6
- windows_aarch64_gnullvm@0.53.1
- windows_aarch64_msvc@0.42.2
- windows_aarch64_msvc@0.52.6
- windows_aarch64_msvc@0.53.1
- windows_i686_gnu@0.42.2
- windows_i686_gnu@0.52.6
- windows_i686_gnu@0.53.1
- windows_i686_gnullvm@0.52.6
- windows_i686_gnullvm@0.53.1
- windows_i686_msvc@0.42.2
- windows_i686_msvc@0.52.6
- windows_i686_msvc@0.53.1
- windows_x86_64_gnu@0.42.2
- windows_x86_64_gnu@0.52.6
- windows_x86_64_gnu@0.53.1
- windows_x86_64_gnullvm@0.42.2
- windows_x86_64_gnullvm@0.52.6
- windows_x86_64_gnullvm@0.53.1
- windows_x86_64_msvc@0.42.2
- windows_x86_64_msvc@0.52.6
- windows_x86_64_msvc@0.53.1
- wit-bindgen@0.51.0
- yasna@0.5.2
- zerocopy@0.8.39
- zeroize@1.8.2
- zeroize_derive@1.4.3

```
Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

## Apache-2.0 WITH LLVM-exception

Used by:
- linux-raw-sys@0.11.0
- rustix@1.1.3
- wasi@0.11.1+wasi-snapshot-preview1
- wasip2@1.0.2+wasi-0.2.9
- wasip3@0.4.0+wasi-0.3.0-rc-2026-01-06
- wit-bindgen@0.51.0

```
Licensed under the Apache License, Version 2.0 with LLVM Exceptions
(the "License"); you may not use this file except in compliance with
the License. You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

With the LLVM Exception:

    As an exception, if, as a result of your compiling your source code,
    portions of this Software are included in a machine-executable object
    form of such source code, you may redistribute such portions in that
    object code form without including the license text.
```

## BSD-1-Clause

Used by:
- fiat-crypto@0.2.9

```
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions and the following disclaimer.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.
```

## BSD-2-Clause

Used by:
- zerocopy@0.8.39

```
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.
```

## BSD-3-Clause

Used by:
- curve25519-dalek@4.1.3
- ed25519-dalek@2.2.0
- encoding_rs@0.8.35
- instant@0.1.13
- matchit@0.8.4
- subtle@2.6.1
- x25519-dalek@2.0.1

```
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.
```

## BSL-1.0

Used by:
- ryu@1.0.23

```
Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## CC0-1.0

Used by:
- notify@7.0.0

```
Creative Commons Legal Code — CC0 1.0 Universal

The person who associated a work with this deed has dedicated the work to
the public domain by waiving all of his or her rights to the work worldwide
under copyright law, including all related and neighboring rights, to the
extent allowed by law.

You can copy, modify, distribute and perform the work, even for commercial
purposes, all without asking permission.

Full text: https://creativecommons.org/publicdomain/zero/1.0/legalcode
```

## CDLA-Permissive-2.0

Used by:
- webpki-root-certs@1.0.6
- webpki-roots@1.0.6

```
Community Data License Agreement — Permissive, Version 2.0

This is a permissive license for data. You are free to use, modify, and
share the data. When you share the data, you must include this license or
provide a link to it, and you must not restrict others from doing anything
this license permits.

Full text: https://cdla.dev/permissive-2-0/
```

## ISC

Used by:
- hyper-rustls@0.27.7
- inotify-sys@0.1.5
- inotify@0.10.2
- ring@0.17.14
- rustls-native-certs@0.8.3
- rustls-webpki@0.103.9
- rustls@0.23.36
- simple_asn1@0.6.3
- untrusted@0.9.0

```
Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

## LGPL-2.1-or-later

Used by:
- r-efi@5.3.0

```
GNU Lesser General Public License v2.1 or later

This library is free software; you can redistribute it and/or modify it
under the terms of the GNU Lesser General Public License as published by
the Free Software Foundation; either version 2.1 of the License, or (at
your option) any later version.

Note: r-efi is also available under MIT and Apache-2.0. BetCode uses the
MIT/Apache-2.0 licensing for this crate.

Full text: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
```

## MIT

Used by:
- aead@0.5.2
- aho-corasick@1.1.4
- allocator-api2@0.2.21
- anstream@0.6.21
- anstyle-parse@0.2.7
- anstyle-query@1.1.5
- anstyle-wincon@3.0.11
- anstyle@1.0.13
- anyhow@1.0.101
- argon2@0.5.3
- async-stream-impl@0.3.6
- async-stream@0.3.6
- async-trait@0.1.89
- atoi@2.0.0
- atomic-waker@1.1.2
- autocfg@1.5.0
- axum-core@0.5.6
- axum@0.8.8
- base16ct@0.2.0
- base64@0.22.1
- base64ct@1.8.3
- bitflags@1.3.2
- bitflags@2.10.0
- blake2@0.10.6
- block-buffer@0.10.4
- bumpalo@3.19.1
- bytes@1.11.1
- castaway@0.2.4
- cc@1.2.55
- cesu8@1.1.0
- cfg-if@1.0.4
- cfg_aliases@0.2.1
- chacha20@0.10.0
- chacha20@0.9.1
- chacha20poly1305@0.10.1
- cipher@0.4.4
- clap@4.5.58
- clap_builder@4.5.58
- clap_derive@4.5.55
- clap_lex@1.0.0
- colorchoice@1.0.4
- combine@4.6.7
- compact_str@0.9.0
- concurrent-queue@2.5.0
- console@0.16.2
- const-oid@0.9.6
- convert_case@0.10.0
- core-foundation-sys@0.8.7
- core-foundation@0.10.1
- core-foundation@0.9.4
- cpufeatures@0.2.17
- cpufeatures@0.3.0
- crc-catalog@2.4.0
- crc@3.4.0
- crossbeam-queue@0.3.12
- crossbeam-utils@0.8.21
- crossterm@0.29.0
- crossterm_winapi@0.9.1
- crypto-bigint@0.5.5
- crypto-common@0.1.7
- curve25519-dalek-derive@0.1.1
- darling@0.23.0
- darling_core@0.23.0
- darling_macro@0.23.0
- der@0.7.10
- deranged@0.5.6
- derive_more-impl@2.1.1
- derive_more@2.1.1
- dialoguer@0.12.0
- digest@0.10.7
- dirs-sys@0.5.0
- dirs@6.0.0
- displaydoc@0.2.5
- document-features@0.2.12
- dotenvy@0.15.7
- ecdsa@0.16.9
- ed25519@2.2.3
- either@1.15.0
- elliptic-curve@0.13.8
- encode_unicode@1.0.0
- encoding_rs@0.8.35
- equivalent@1.0.2
- errno@0.3.14
- event-listener@5.4.1
- fastrand@2.3.0
- ff@0.13.1
- fiat-crypto@0.2.9
- filetime@0.2.27
- find-msvc-tools@0.1.9
- fixedbitset@0.5.7
- flume@0.11.1
- fnv@1.0.7
- form_urlencoded@1.2.2
- fsevent-sys@4.1.0
- futures-channel@0.3.31
- futures-core@0.3.31
- futures-executor@0.3.31
- futures-intrusive@0.5.0
- futures-io@0.3.31
- futures-macro@0.3.31
- futures-sink@0.3.31
- futures-task@0.3.31
- futures-util@0.3.31
- generic-array@0.14.7
- getrandom@0.2.17
- getrandom@0.3.4
- getrandom@0.4.1
- group@0.13.0
- h2@0.4.13
- hashbrown@0.15.5
- hashbrown@0.16.1
- hashlink@0.10.0
- heck@0.5.0
- hex@0.4.3
- hkdf@0.12.4
- hmac@0.12.1
- http-body-util@0.1.3
- http-body@1.0.1
- http@1.4.0
- httparse@1.10.1
- httpdate@1.0.3
- hyper-rustls@0.27.7
- hyper-timeout@0.5.2
- hyper-util@0.1.20
- hyper@1.8.1
- ident_case@1.0.1
- idna@1.1.0
- idna_adapter@1.2.1
- indexmap@2.13.0
- indoc@2.0.7
- inout@0.1.4
- instability@0.3.11
- ipnet@2.11.0
- iri-string@0.7.10
- is_terminal_polyfill@1.70.2
- itertools@0.14.0
- itoa@1.0.17
- jni-sys@0.3.0
- jni@0.21.1
- js-sys@0.3.85
- jsonwebtoken@10.3.0
- kasuari@0.4.11
- kqueue-sys@1.0.4
- kqueue@1.1.1
- lazy_static@1.5.0
- libc@0.2.180
- libm@0.2.16
- libredox@0.1.12
- libsqlite3-sys@0.30.1
- line-clipping@0.3.5
- linux-raw-sys@0.11.0
- litrs@1.0.0
- lock_api@0.4.14
- log@0.4.29
- lru@0.16.3
- matchers@0.2.0
- matchit@0.8.4
- memchr@2.8.0
- mime@0.3.17
- mio@1.1.1
- multimap@0.10.1
- nix@0.31.1
- notify-types@1.0.1
- nu-ansi-term@0.50.3
- num-bigint-dig@0.8.6
- num-bigint@0.4.6
- num-conv@0.2.0
- num-integer@0.1.46
- num-iter@0.1.45
- num-traits@0.2.19
- num_threads@0.1.7
- once_cell@1.21.3
- once_cell_polyfill@1.70.2
- opaque-debug@0.3.1
- openssl-probe@0.2.1
- p256@0.13.2
- p384@0.13.1
- parking@2.2.1
- parking_lot@0.12.5
- parking_lot_core@0.9.12
- password-hash@0.5.0
- pem-rfc7468@0.7.0
- pem@3.0.6
- percent-encoding@2.3.2
- petgraph@0.8.3
- pin-project-internal@1.1.10
- pin-project-lite@0.2.16
- pin-project@1.1.10
- pin-utils@0.1.0
- pkcs1@0.7.5
- pkcs8@0.10.2
- pkg-config@0.3.32
- poly1305@0.8.0
- powerfmt@0.2.0
- ppv-lite86@0.2.21
- prettyplease@0.2.37
- primeorder@0.13.6
- proc-macro2@1.0.106
- pulldown-cmark@0.13.0
- quote@1.0.44
- r-efi@5.3.0
- rand@0.10.0
- rand@0.8.5
- rand_chacha@0.3.1
- rand_core@0.10.0
- rand_core@0.6.4
- ratatui-core@0.1.0
- ratatui-crossterm@0.1.0
- ratatui-macros@0.7.0
- ratatui-widgets@0.3.0
- ratatui@0.30.0
- rcgen@0.14.7
- redox_syscall@0.5.18
- redox_users@0.5.2
- regex-automata@0.4.14
- regex-syntax@0.8.9
- regex@1.12.3
- reqwest@0.13.2
- rfc6979@0.4.0
- rsa@0.9.10
- rustc_version@0.4.1
- rustix@1.1.3
- rustls-native-certs@0.8.3
- rustls-pki-types@1.14.0
- rustls-platform-verifier-android@0.1.1
- rustls-platform-verifier@0.6.2
- rustls@0.23.36
- rustversion@1.0.22
- same-file@1.0.6
- schannel@0.1.28
- scopeguard@1.2.0
- sec1@0.7.3
- security-framework-sys@2.15.0
- security-framework@3.5.1
- semver@1.0.27
- serde@1.0.228
- serde_core@1.0.228
- serde_derive@1.0.228
- serde_json@1.0.149
- serde_path_to_error@0.1.20
- serde_spanned@0.6.9
- serde_urlencoded@0.7.1
- sha2@0.10.9
- sharded-slab@0.1.7
- shell-words@1.1.1
- shlex@1.3.0
- signal-hook-mio@0.2.5
- signal-hook-registry@1.4.8
- signal-hook@0.3.18
- signature@2.2.0
- slab@0.4.12
- smallvec@1.15.1
- socket2@0.6.2
- spin@0.9.8
- spki@0.7.3
- sqlx-core@0.8.6
- sqlx-macros-core@0.8.6
- sqlx-macros@0.8.6
- sqlx-sqlite@0.8.6
- sqlx@0.8.6
- stable_deref_trait@1.2.1
- static_assertions@1.1.0
- strsim@0.11.1
- strum@0.27.2
- strum_macros@0.27.2
- syn@2.0.114
- synstructure@0.13.2
- system-configuration-sys@0.6.0
- system-configuration@0.7.0
- tempfile@3.25.0
- thiserror-impl@1.0.69
- thiserror-impl@2.0.18
- thiserror@1.0.69
- thiserror@2.0.18
- thread_local@1.1.9
- time-core@0.1.8
- time-macros@0.2.27
- time@0.3.47
- tokio-macros@2.6.0
- tokio-rustls@0.26.4
- tokio-stream@0.1.18
- tokio-util@0.7.18
- tokio@1.49.0
- toml@0.8.23
- toml_datetime@0.6.11
- toml_edit@0.22.27
- toml_write@0.1.2
- tonic-build@0.14.3
- tonic-health@0.14.3
- tonic-prost-build@0.14.3
- tonic-prost@0.14.3
- tonic@0.14.3
- tower-http@0.6.8
- tower-layer@0.3.3
- tower-service@0.3.3
- tower@0.5.3
- tracing-attributes@0.1.31
- tracing-core@0.1.36
- tracing-log@0.2.0
- tracing-serde@0.2.0
- tracing-subscriber@0.3.22
- tracing@0.1.44
- try-lock@0.2.5
- typenum@1.19.0
- unicase@2.9.0
- unicode-ident@1.0.23
- unicode-segmentation@1.12.0
- unicode-truncate@2.0.1
- unicode-width@0.2.2
- universal-hash@0.5.1
- url@2.5.8
- utf8_iter@1.0.4
- utf8parse@0.2.2
- uuid@1.20.0
- vcpkg@0.2.15
- version_check@0.9.5
- walkdir@2.5.0
- want@0.3.1
- wasi@0.11.1+wasi-snapshot-preview1
- wasip2@1.0.2+wasi-0.2.9
- wasip3@0.4.0+wasi-0.3.0-rc-2026-01-06
- wasm-bindgen-futures@0.4.58
- wasm-bindgen-macro-support@0.2.108
- wasm-bindgen-macro@0.2.108
- wasm-bindgen-shared@0.2.108
- wasm-bindgen@0.2.108
- web-sys@0.3.85
- winapi-i686-pc-windows-gnu@0.4.0
- winapi-util@0.1.11
- winapi-x86_64-pc-windows-gnu@0.4.0
- winapi@0.3.9
- windows-link@0.2.1
- windows-registry@0.6.1
- windows-result@0.4.1
- windows-strings@0.5.1
- windows-sys@0.45.0
- windows-sys@0.52.0
- windows-sys@0.60.2
- windows-sys@0.61.2
- windows-targets@0.42.2
- windows-targets@0.52.6
- windows-targets@0.53.5
- windows_aarch64_gnullvm@0.42.2
- windows_aarch64_gnullvm@0.52.6
- windows_aarch64_gnullvm@0.53.1
- windows_aarch64_msvc@0.42.2
- windows_aarch64_msvc@0.52.6
- windows_aarch64_msvc@0.53.1
- windows_i686_gnu@0.42.2
- windows_i686_gnu@0.52.6
- windows_i686_gnu@0.53.1
- windows_i686_gnullvm@0.52.6
- windows_i686_gnullvm@0.53.1
- windows_i686_msvc@0.42.2
- windows_i686_msvc@0.52.6
- windows_i686_msvc@0.53.1
- windows_x86_64_gnu@0.42.2
- windows_x86_64_gnu@0.52.6
- windows_x86_64_gnu@0.53.1
- windows_x86_64_gnullvm@0.42.2
- windows_x86_64_gnullvm@0.52.6
- windows_x86_64_gnullvm@0.53.1
- windows_x86_64_msvc@0.42.2
- windows_x86_64_msvc@0.52.6
- windows_x86_64_msvc@0.53.1
- winnow@0.7.14
- wit-bindgen@0.51.0
- yasna@0.5.2
- zerocopy@0.8.39
- zeroize@1.8.2
- zeroize_derive@1.4.3
- zmij@1.0.20

```
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## MPL-2.0

Used by:
- nucleo-matcher@0.3.1
- option-ext@0.2.0

```
Mozilla Public License Version 2.0

This Source Code Form is subject to the terms of the Mozilla Public License,
v. 2.0. If a copy of the MPL was not distributed with this file, You can
obtain one at https://mozilla.org/MPL/2.0/.

Key obligations:
- Modified MPL-2.0 files must remain under MPL-2.0
- MPL-2.0 source files must be made available to recipients
- Larger works may combine MPL-2.0 code with other licenses
- File-level copyleft (not project-level)

Full text: https://mozilla.org/MPL/2.0/
```

## Unicode-3.0

Used by:
- icu_collections@2.1.1
- icu_locale_core@2.1.1
- icu_normalizer@2.1.1
- icu_normalizer_data@2.1.1
- icu_properties@2.1.2
- icu_properties_data@2.1.2
- icu_provider@2.1.1
- litemap@0.8.1
- potential_utf@0.1.4
- tinystr@0.8.2
- unicode-ident@1.0.23
- writeable@0.6.2
- yoke-derive@0.8.1
- yoke@0.8.1
- zerofrom-derive@0.1.6
- zerofrom@0.1.6
- zerotrie@0.2.3
- zerovec-derive@0.11.2
- zerovec@0.11.5

```
Unicode License v3

Permission is hereby granted, free of charge, to any person obtaining a copy
of data files and any associated documentation (the "Data Files") or software
and any associated documentation (the "Software") to deal in the Data Files
or Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, and/or sell copies of the
Data Files or Software.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED.

Full text: https://www.unicode.org/license.txt
```

## Unlicense

Used by:
- aho-corasick@1.1.4
- memchr@2.8.0
- same-file@1.0.6
- walkdir@2.5.0
- winapi-util@0.1.11

```
This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or distribute
this software, either in source code form or as a compiled binary, for any
purpose, commercial or non-commercial, and by any means.

Full text: https://unlicense.org/
```

## Zlib

Used by:
- foldhash@0.1.5
- foldhash@0.2.0

```
This software is provided 'as-is', without any express or implied warranty.
In no event will the authors be held liable for any damages arising from the
use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software.
2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.
3. This notice may not be removed or altered from any source distribution.
```

