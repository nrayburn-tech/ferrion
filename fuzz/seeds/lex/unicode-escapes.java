class Escapes {
  // A line comment ended by an escape: \u000a int afterComment;
  String s = "" + \uuuu005c\ + \n";
  String pair = "😀";
  char lone = '\uD800';
  int été = 1;
  /* a comment */
  String notEscaped = "\\u0041";
  int bad = \u00G1 + \u;
  String split = "ok";
  String é = "Ω 😀 \u{1F600}";
}
