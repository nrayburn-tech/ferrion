// Array initializers whose input has the elements in rows: GJF keeps them when every row has the
// same number, the last may have fewer, at least two rows are full, the first row has two
// elements other than arrays, and the rows' first elements are of one kind, as are most of
// each other column's. A row that doesn't fit breaks at +4 where an element doesn't fit.
class ArrayRows {
  String[] wide = {
    "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc",
    "dddddddddd", "eeeeeeeeee", "ffffffffff",
    "gggggggggg", "hhhhhhhhhh", "iiiiiiiiii",
  };
  String[] lastShorter = {
    "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc",
    "dddddddddd", "eeeeeeeeee", "ffffffffff",
    "gggggggggg"
  };
  int[] narrow = {
    1, 2, 3,
    4, 5, 6,
    7, 8, 9,
    10, 11, 12
  };
  String[] fits = {
    "a", "b", "c",
    "d", "e", "f"
  };
  int[] twoFull = {
    1, 2,
    3, 4,
    5
  };
  int[] wrapped = {
    1, 2, 3,
    4, 5,
        6,
    7, 8, 9
  };
  Object[] columns = {
    "a", A, 1,
    "b", B, 2,
    "c", f(), 3
  };
  Object[] negative = {
    -1, -2, -3,
    4, -5, -6,
    -7
  };
  Object[] unary = {
    !a, !b, !c,
    d, !e, !f,
    !g
  };
  Object[] members = {
    a.b, A.class, this.c,
    d.e, f.g, h.i
  };
  int[] comments = {
    1, 2, 3, // one
    // between
    4, /* x */ 5, 6, /* y */
    7, 8, 9 // three
  };
  int[] blankLine = {
    1, 2, 3,

    4, 5, 6
  };
  String[] tooLong = {
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "cccc", "dddd",
    "eeee", "ffff", "gggg", "hhhh",
    "iiii", "jjjj", "kkkk", "llll"
  };
  String[] commentTooLong = {
    "aaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "ccccccccccccccc", // a comment
    "aaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "ccccccccccccccc",
  };
  Object[] textBlock = {
    "a", 1,
    """
    x
    """, 2,
    "a", 3
  };
  int[][] nested = {
    {1, 2}, {3, 4}, {5, 6},
    {7, 8}, {9, 10}, {11, 12}
  };
  int[][] onePerLine = {
    {0, 0},
    {0, 1},
    {1}
  };
  Object[] onePerLineNew = {
    new int[] {0, 0},
    new int[] {0, 1}
  };
  int[][][] deeper = {
    {{1}, {2}},
    {{3}},
  };

  // Not kept.
  String[] middleShorter = {
    "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc",
    "dddddddddd", "eeeeeeeeee",
    "gggggggggg", "hhhhhhhhhh", "iiiiiiiiii"
  };
  String[] firstLonger = {
    "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc", "dddddddddd",
    "eeeeeeeeee", "ffffffffff",
    "gggggggggg", "hhhhhhhhhh"
  };
  String[] oneFull = {
    "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc",
    "dddddddddd"
  };
  String[] onBraceLine = {"aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc",
    "dddddddddd", "eeeeeeeeee", "ffffffffff"};
  String[] rowsOfOne = {
    "aaaaaaaaaa",
    "bbbbbbbbbb",
    "cccccccccc"
  };
  int[] indentedLeft = {
    1, 2, 3,
  4, 5, 6,
    7, 8, 9
  };
  int[] indentedRight = {
    1, 2, 3,
    4, 5, 6,
      7, 8, 9
  };
  Object[] firstColumn = {
    "a", A, 1,
    "b", B, 2,
    C
  };
  Object[] secondColumn = {
    a, b, c,
    d, f(), e,
    g
  };
  Object[] operators = {
    a + 1, b + 2, c + 3,
    d - 4, e + 5, f + 6
  };
  Object[] literals = {
    1, 2, 3,
    4L, 5, 6
  };
  Object[] twoUnary = {
    a, b, c,
    !!d, e, f,
    g
  };
  Object[] parenthesized = {
    a, b, c,
    (d), e, f
  };
  int[][] singleElements = {
    {1},
    {2},
    {3, 4}
  };
  int[][] empty = {
    {}, {},
    {1, 2}, {3, 4}
  };
  int[][] mixedWithNull = {
    {0, 0},
    null,
    {1, 0}
  };
}
