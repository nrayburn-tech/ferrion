class Literals {
  int i = 0 + 7 + 0_7 + 0x7f_FF + 0B1010 + 2147483647;
  long l = 9223372036854775807L + 0xFFl + 0b1L + 0777L;
  float f = 1f + 1.5F + .5e-3f + 0x1.8p1f + 1e10f;
  double d = 1.0 + 1. + .5 + 1e+3 + 1E-3d + 0x1p-1074 + 0x.8P2D + 1_000.000_1;
  char c = 'a', q = '\'', b = '\\', o = '\0', m = '\377', u = 'A', s = '\s';
  String str = "tab\t newline\n quote\" octal\12 \\ \b\f\r";
  String empty = "";
  String block = """
      Hello, "text" block!
        indented \
      continued\s
      \"""
      """;
  String oneLine = """
      x""";
  boolean t = true, n = false;
  Object z = null;
}
