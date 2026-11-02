// Rows whose first item follows the opening delimiter: the rows after it must start at its
// column, counting a tab as one column, a comment before the first item as its start, and
// each operator before it on its line as wide as the square of its length, as GJF counts.
class ListColumns {
  String[] k = {
    "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc",
    "dddddddddd", "eeeeeeeeee", "ffffffffff"
  };
  String[] l = {
    "aaaaaaaaaa", "bbbbbbbbbb",
    "cccccccccc", "dddddddddd",
    "eeeeeeeeee"
  };
  String[] notUnder = {
    "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc", "dddddddddd", "eeeeeeeeee", "ffffffffff"
  };

  @A({
    "a, b", "c, d",
    "e, f", "g, h"
  })
  void m() {
    map =
        Map.of(
            "breakfast", List.of("toast"),
            "lunch", List.of("toast", "cereal"));
    foo(
        Map.of(
            "breakfast", A,
            "lunch", B));
    foo(/* c */ "a", A, "b", B);
    foo(
        /* c */ "a", A,
        "b", B);
    x = foo("a", A, "b", B);
    x =
        foo(
            "a", A,
            "b", B);
    Map<String, List<String>> m = Map.of("breakfast", A, "lunch", B);
    Map<String, List<String>> m =
        Map.of(
            "breakfast", A,
            "lunch", B);
    List<List<List<String>>> m =
        Map.of(
            "breakfast", A,
            "lunch", B);
    m >>>=
        Map.of(
            "breakfast", A,
            "lunch", B);
    f(x ->
        Map.of(
            "breakfast", A,
            "lunch", B));
    f((int... x) ->
        Map.of(
            "breakfast", A,
            "lunch", B));
    m =
        A::b == null
            ? null
            : Map.of(
                "breakfast", A,
                "lunch", B);
    m =
        a < b
            ? null
            : Map.of(
                "breakfast", A,
                "lunch", B);
  }
}
