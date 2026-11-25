# Public dense fixed8 checked-bounded Jobs proof

The Linode correctness SETUP released with exit 0 at 04:13:12 UTC on November 25, 2026. All 11 fault, wire and teardown cases passed on production revision `3c4032d06b10bc54a47290c673562fb063b47789`. The proof ELF SHA-256 is `4924b5880247059032786f43f490cf99bde8a0a69adc790994ee01a51ab91058`. The grant covered correctness only.

The exact request is in `jobs.json`. The raw export, fenced release and verified replay are under `evidence/native`. The adjacent controller source packet and its verifier remain in the implementation checkout. From that checkout, run `python3 -B proof/jobs-group-success/native.py replay target/public-dense-fixed8/final-setup RELEASE RAW`, using the archived release and raw paths.
