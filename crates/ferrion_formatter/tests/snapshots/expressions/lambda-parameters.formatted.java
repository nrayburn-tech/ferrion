class LambdaParameters {
  void f() {
    Selector selector =
        (ctx, preamble, cookies, outputBuffer, threadId, localOverrides, peerParameters, mapper) ->
            new Answer(true, handler);
    Selector s2 =
        (Context ctx, Preamble preamble, Cookies cookies, OutputBuffer outputBuffer, int id) ->
            new Answer(true, handler);
    call(
        aaaaaaaaaaaaaaaaaaaa,
        (ScopedReleaser keeper, FakeResources v1, FakeResources v2, String v3, String v4) -> {
          keeper.close();
        });
    foo.bar()
        .call(
            (ScopedReleaser keeper,
                FakeResources v1,
                FakeResources v2,
                String v3,
                String v4,
                String v5) -> {
              keeper.close();
            });
    Selector s3 =
        (Context ctx, Preamble preamble, Cookies cookies, OutputBuffer outputBuffer, int id) -> {
          return null;
        };
    Selector s4 =
        (aaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, ccccccccccccccccccccccccccccccc) ->
            aaaaaaaaaaaaaaaaaaaa;
    Selector s5 =
        (final Context ctx, Preamble preamble, Cookies cookies, OutputBuffer outputBuffer) -> null;
    Selector s6 =
        (a, // c
            b) -> null;
  }
}
