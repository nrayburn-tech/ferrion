class Escapes {
  // A line comment ended by an escape: \u000a int afterComment;
  String s = "\u0022 + \uuuu005c\u005c + \n";
  String pair = "\uD83D\uDE00";
  char lone = '\uD800';
  int \u00e9t\u00e9 = 1;
  /* a comment */
  String notEscaped = "\\u0041";
  int bad = \u00G1 + \u;
  String split = "ok";
  String é = "Ω 😀 \u{1F600}";
}
