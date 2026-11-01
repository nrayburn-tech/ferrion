// Comments inside types.
class Comments {
  List</* c */ String> a;
  List<String /* c */> b;
  Map<String, /* c */ Integer> c;
  Map<String /* c */, Integer> d;
  List<? /* c */ extends T> e;
  java.util./* c */ List<String> f;
  java.util /* c */.List<String> g;
  int /* c */[] h;
}

class Leading<
    // c
    T, U> {}

class Trailing<T, // c
    U> {}

class Bounds<T extends /* c */ A & /* d */ B> {}
