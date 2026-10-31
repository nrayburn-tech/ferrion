class Malformed {
  int a = 0x + 0b + 1e + 1_ + 0x1p + 08 + 1__0 + 0_x1 + 1.e;
  char c = '' + 'ab' + '\q';
  String s = "\x # ` \
  String t = """not on its own line""";
  Object o = #;
}
/* unterminated
