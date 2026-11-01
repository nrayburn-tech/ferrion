// Array types and dimensions, and varargs.
class Arrays {
  int [ ] a;
  int[] [] b;
  String @A [] c;
  int @A [] @B [] d;
  int @A(1) [] [] @B [] e;
  int f [];
  int g @A [];
  int h[], i [] [];
  List<String> [] j;
  Class<?>[] k;
  Map<String,int[]> l;
  List<int @A []> m;
  Map.Entry<String, Integer>[][] n;

  void o(String ... p) {}

  void q(int[]... r) {}

  void s(List<String>@A... t) {}

  int[] u() [] {
    return null;
  }
}
