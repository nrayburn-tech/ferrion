// Arguments whose input has them in pairs, one pair a line: GJF keeps them only in pairs, every
// pair full, with the first items of one kind and most second items of one kind.
class ListPairs {
  void m() {
    a =
        foo(
            aaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbb,
            ccccccccccccccccccccccccc, ddddddddddddddddddddddd,
            eeeeeeeeeeeeeeeeee, fffffffffffffffff);
    b =
        foo(
            "a", f(A),
            "b", f(B),
            "c", f(C));
    c =
        foo(
            "a", -A,
            "b", !B);
    d =
        foo(
            "a", A,
            "b", B);
    e =
        foo(
            "a", A, // one
            // between
            "b", B); // two
    f =
        foo(
            "a", A,
            "b", B,
            "c", 1);

    // Not kept.
    g = foo(1, 2, 3, 4, 5, 6, 7, 8, 9);
    h = foo("a", A, "b", B, "c");
    i = foo("a", A, "b", 1);
    j = foo("a", A, 1, B);
    k = foo("a", f(A), "b", B + 1, "c", new C());
    l = foo("a", A, "b", B, "c", C);
  }
}
