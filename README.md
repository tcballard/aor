# AoR

A from-scratch Rust web framework for Omarchy. Built from the socket up, with typed boundaries and verification as its spine.

Implementation follows the product and technical specification v0.3. This repository is under active development; it is not ready for public application traffic.

## The honesty clause

1. Every layer here has a mature crate that does it better today. AoR exists because building it is the point.
2. The HTTP parser, session handling and CSRF implementation are the author's and have had the review described in the implementation evidence and no more. Deploy behind a reverse proxy; do not put it on the public internet without one.
3. If a security defect is found in a layer the author cannot fix within a week, that layer is replaced by the crate it displaced and the spec is amended. Pride is not a release gate.

AoR is a working acronym. No crate or domain has been reserved. Packages remain unpublished while implementation and release evidence are developed.
