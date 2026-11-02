// Arguments after a format string, a first argument that is a string literal with a `%`, or a
// concatenation of such literals: when the arguments break, the format string is on a line of
// its own and the rest are arguments of their own, all on the next line, filled, or one per line.
class ListFormatStrings {
  void m() {
    foo(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa %s bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        xxxxx, yyyyy);
    throw new StorageExhaustedException(
        "Ran out of memory while growing the slot table: from %,d to %,d",
        e, this.size + 1, slotCount);
    String.format(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa %s "
            + "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        xxxxx, yyyyy);
    foooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooo(
        "a %s", xxxxxxxxx, yyyyyyyyyyy);
    foo(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa %s",
        a, b, c, d, e, f, g, h, i, j, k, l, m, n);
    foo(
        "%s",
        a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w, x, y, z, aa, bb, cc,
        dd);
    foo(
        "%s",
        aaaaaaaaaaaaaaaaaa,
        bbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
        ccccccccccccccc,
        dddddddddddddddddddddddd);
    foo(
        "%s",
        xxxxxxxxxxxxxxxxxxxxxxxxxxxx, // c
        yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy,
        zzzzzzzzzzzzzzzzzzzzzzz);
    foo(
        // c
        "%s",
        xxxxxxxxxxxxxxxxxxxxxxxxxxxx, yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy, zzzzzzzzzzzzzzzzzzzzzzz);
    foo(
        """
        %s
        """,
        xxxxxxxx, yyyyyyyyy);
    foo(
        "%s",
        () -> {
          run();
        });

    // Not format strings.
    foo(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa {} bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        xxxxx,
        yyyyy);
    foo(
        zzzz,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa %s bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        xxxxx,
        yyyyy);
    foo(
        "%s" + x,
        xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx,
        yyyyyyyyyyyyyyyyyyyyyyyyyyyyyy,
        zzzzzzzzzzzzzzzzzzzzzz);
    foo(
        ("%s"),
        xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx,
        yyyyyyyyyyyyyyyyyyyyyyyyyyyyyy,
        zzzzzzzzzzzzzzzzzzzzzz);
    foo(
        '%',
        xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx,
        yyyyyyyyyyyyyyyyyyyyyyyyyyyyyy,
        zzzzzzzzzzzzzzzzzzzzzz);
    foo(
        "%s", a,
        "%d", b);
  }
}
