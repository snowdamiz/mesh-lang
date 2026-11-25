# Fixed8 and logical-key factorial diagnostic stop

The controller released the prospective factorial booking with exit 1
at 03:43:33 UTC. Its first stock diagnostic exited 0 and exported a
1,024-job ledger. Both the ledger and row report two runners and 64
deployment slots. The sealed scorer stopped at its assertion that
those deployment fields equal 32.

The controller stopped before the composed diagnostic and before either
untraced timing group. Replay selected no complete groups and closed
peer eligibility. The original grouped-refill and logical-key evidence
remains separately archived.
[Release](controller-release.json) · [Sealed replay](replay.json) · [Raw stock diagnostic](blocks)
