// Array creation, access, and initializers: an initializer of elements all at most 9 wide is
// filled, otherwise one element per line, and a trailing comma keeps its elements off the
// braces' lines.
class Arrays {
  void m() {
    x = new int[3];
    x = new int[3][4][];
    x = new @A int @B [3];
    x = new Object[] {};
    x = new Object[][] {{}};
    x =
        new int[] {
          1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
          26, 27
        };
    x =
        new String[] {
          "aaaaaaaaaaaaaaaaaa",
          "bbbbbbbbbbbbbbbbbbbbbbbbb",
          "cccccccccccccccccccccccccc",
          "ddddddddddddddd"
        };
    x =
        new int[][] {
          {1, 2}, {3, 4}, {5, 6}, {7, 8}, {9, 10}, {11, 12}, {13, 14}, {15, 16}, {17, 18}, {19, 20}
        };
    x =
        new Object[] {
          aaa, bbb, ccc, ddd, eee, fff, ggg, hhh, iii, jjj, kkk, lll, mmm, nnn, abcdefghij, ppp
        };
    x =
        new Object[] {
          fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a, b), fo(a)
        };
    x =
        new int[] {
          1, 2,
        };
    x =
        new int[] {
          1, 2, 3, // trailing 3
          4, 5, 6
        };
    x =
        new int[] {
          // leading the first element
          1, 2, 3
        };
    x =
        new int[] {
          // only a comment
        };
    foo(new int[] {1, 2, 3});
    foo(
        new String[] {
          "aaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbb", "cccccccccccccccccccccccccc", "dd"
        });
    foo(
        a,
        new int[] {
          1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24
        });
    x =
        a[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa][
            bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb];
    x =
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa[
            bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccccccccccc];
  }
}
