// Local variable declarations: modifiers and annotations, several declarators, and long types.
class LocalVariables {
  void m() {
    int x;
    int x = 1, y = 2, z;
    final int x = 1;
    var x = 1;
    Map<String, List<Integer>> m = new HashMap<>();
    @Ann int x = 1;
    @Ann
    int x = 1;
    @Ann final int x = 1;
    @SuppressWarnings("unchecked") List<String> list = x;
    int aaaaaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, cccccccccccccccccccccccccccccccc;
    String a = b, ccccccccccccccccccccccccccccccc = dddddddddddddddddddddddddddddddddddddddddddddddddddd;
    final SomeVeryLongTypeName<WithTypeArguments, AndMoreTypeArguments> someVeryLongVariableNameXYZW;
    class Local {}
    record R(int a) {}
  }
}
