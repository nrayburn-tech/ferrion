// Arguments: all on the call's line, otherwise all on the next line at +4, otherwise one per
// line; filled when every argument is at most 9 wide, and in pairs when the input has them so.
class Arguments {
  void m() {
    foo();
    foo(a);
    foo(argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeCCCCCCC);
    foobarbaz(
        argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeC, d);
    foo(
        argumentNumberOneAAAAAAAAAAA,
        argumentNumberTwoBBBBBBBBBBBBB,
        argumentNumberThreeCCCCCCC,
        dd);
    someObject.someMethodName(
        argumentNumberOneAAAAAAAAAAA,
        bar(
            argumentNumberTwoBBBBBBBBBBBBB,
            argumentNumberThreeCCCCCCC,
            argumentNumberFourDDDDDDDDD),
        eeeeeeeeeeeeeee);
    foo(
        bar(
            baz(
                argumentNumberOneAAAAAAAAAAA,
                argumentNumberTwoBBBBBBBBBBBBB,
                argumentNumberThreeCCCCCCC)));
    foo(
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa);
    foo(
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28);
    foo(
        a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w, x, y, z, aa, bb, cc,
        dd, ee);
    foo(
        aaa,
        bbb,
        ccc,
        ddd,
        eee,
        fff,
        ggg,
        hhh,
        iii,
        jjj,
        kkk,
        lll,
        mmm,
        nnn,
        ooo,
        ppp,
        abcdefghij,
        rrr);
    foo(
        fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b),
        fo(a, b));
    this(
        argumentNumberOneAAAAAAAAAAA,
        argumentNumberTwoBBBBBBBBBBBBB,
        argumentNumberThreeCCCCCCCCCCCCCCC);
    outer.super(
        argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeCCCCCCC);
    LOG.info(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa {} bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        x,
        y);
    foo(
        String.format(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa %s bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            x));
    foo(/* none */ );
    foo(
        // nothing
        );
    foo(a, /* leads b */ b);
    foo(
        argumentNumberOneAAAAAAAAAAA, /* comment */
        argumentNumberTwoBBBBBBBBBBBBB,
        argumentNumberThreeCCCCCCC,
        dd);
    foo(
        argumentNumberOneAAAAAAAAAAA, // trailing
        argumentNumberTwoBBBBBBBBBBBBB);
    foo(
        // leading
        a, b);
    remap(
        key,
        valueAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,
        /* hints= */ null,
        /* computeIfAbsent= */ true);
    foo(a, b)
        .bar(c) // trailing the call
        .baz();
    x =
        Map.of(
            "magenta", HIGHLIGHT_MAGENTA,
            "teal", HIGHLIGHT_TEAL,
            "amber", HIGHLIGHT_AMBER,
            "red", HIGHLIGHT_RED);
    x =
        Map.of(
            "a", A,
            "b", B);
    x =
        Map.of(
            "magentaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                HIGHLIGHT_MAGENTAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,
            "teal", HIGHLIGHT_TEAL);
    foo(
        """
        text
        """);
    foo(
        a,
        """
        text
        """);
  }
}
