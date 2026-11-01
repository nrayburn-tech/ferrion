// `while`, `do`, `for`, and enhanced `for`, with their bodies and headers.
class Loops {
  void m() {
    while (a) b();
    while (a) {
      b();
    }
    while (a)
      ;
    while (a) while (b) c();
    while (a)
      bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb = c;
    do b();
    while (a);
    do {
      b();
    } while (a);
    do
      ;
    while (a);
    do bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb = c;
    while (a);
    for (; ; )
      ;
    for (; i < n; ) {}
    for (i = 0, j = 1; ; ) {}
    for (int i = 0; i < n; i++) b();
    for (int i = 0, j = 0; i < n; i++, j++) {
      b();
    }
    for (int i = 0; i < n; i++) for (int j = 0; j < n; j++) b();
    for (int iiiiiiiiiiiiiiiiiiiii = 0;
        iiiiiiiiiiiiiiiiiiiii < nnnnnnnnnnnnnnnnnnnnnn;
        iiiiiiiiiiiiiiiiiiiii++) {
      b();
    }
    for (iiiiiiiiiiiiiiiiiiiiiiiiii = 0;
        ;
        iiiiiiiiiiiiiiiiiiiiii++, jjjjjjjjjjjjjjjjjjjjjjjjj++,
            kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk++) {}
    for (;
        ;
        iiiiiiiiiiiiiiiiiiiiii++, jjjjjjjjjjjjjjjjjjjjjjjjj++, kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk++,
            llllllllllllll++) {}
    for (String s : list) b();
    for (final String s : list) {
      b();
    }
    for (@Ann String s : list) {
      b();
    }
    for (Map.Entry<String, SomeVeryLongTypeNameForTesting> entryWithLongName :
        someMapObject.entrySet()) {
      b();
    }
    for (Map.Entry<String, SomeVeryLongTypeNameForTestingXYZWVUTS> entryWithLongNameXYZWVUTSRQPOAB :
        m) {}
  }
}
