class AnnotatedNames {
  void m(String[] a) throws java.io.@A IOException, @B java.lang.@C Error {
    java.util.@A Arrays.stream(a).count();
    a.@A b.c = 1;
    a.@A b.@B c.<T>d();
    a.@A b[0]++;
    for (java.util.@A Arrays.stream(a);;) {
      break;
    }
    Object o = @A Object::new;
    o = @A java.util.List<@B String>::<@C Integer>of;
    o = x @A []::new;
    o = (@A String) o;
  }
}
