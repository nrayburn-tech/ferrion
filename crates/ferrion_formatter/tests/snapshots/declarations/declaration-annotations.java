@SuppressWarnings({"unchecked", "rawtypes"}) public class DeclarationAnnotations {
  @Override public String toString() { return ""; }
  @Override
  @Deprecated public int hashCode() { return 0; }
  @A @B @C void a() {}
  public @A void b() {}
  @A public @B void c() {}
  @A(1) public @B(2) void d() {}
  @A /* c */ void e() {}
  @A // c
  void f() {}
  @A @B DeclarationAnnotations() {}
  @A @B class Inner {}
  @A enum E { X }
  @A static @B class C {}
  @Inject Foo foo;
  @Inject private Foo foo2;
  @A() int x2;
  @A @B @C @D int x3;
  @A
  @B int x4;
  @A private static int x5;
  private @A(1) static int x6;
  @a.b.C int x7;
  int y1;
  @A(1) int y2;
  int y3;
  @A(x = 1) int y4;
  @Mock(answer = RETURNS_DEEP_STUBS) Foo y5;
  @A @B(1) int y6;
  @A(1) private @B int y7;
  int y8;
  @A(1) @B private static final int y9 = 1;
  @A int z1 = computeTheMapFromSomething(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbb);
  int z2;
  @Nullable private final Map<String, Something> fooBarBazzzzzzzzzzzz2 = new HashMap<>(capacityyy);
  int z3;
  void locals() {
    @A int y1 = 1;
    @A final int y2 = 1;
    @A @B int y3 = 1;
    final @A @B int y4 = 1;
    @A() int y5 = 1;
    @SuppressWarnings("unchecked") List<String> l = null;
    @Nullable String xxxxxxxxxxxxxxxxxxxxxxxxxxx = computeTheMapFromSomething(aaaaaaaaaaaaaa, bbbbbbbbb);
  }
}
