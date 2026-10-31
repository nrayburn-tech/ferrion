// releases: 8 14 15 16 17
sealed interface S permits F {}
final class F implements S {
  String t = """
      text
      """;
}
