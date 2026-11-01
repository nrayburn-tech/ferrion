// `assert` with and without a message: the condition breaks from the statement without one,
// and from the `:`'s +4 with one.
class Assert {
  void m() {
    assert entryTotal == (maxEntriesPerBucket + 1) / 2 || entryTotal == (finalBucketEntries + 1) / 2 || entryTotal == 2;
    assert entryTotal == (maxEntriesPerBucket + 1) / 2 || entryTotal == (finalBucketEntries + 1) / 2 || entryTotal == 2 : "message";
    assert entryTotal == (maxEntriesPerBucket + 1) / 2 : "a message that is long enough to break here " + x;
    assert a : "short";
  }
}
