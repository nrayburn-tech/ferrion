// Throws clauses that fit on the method's line.
class Throws {
  void a() throws E, F {}

  void b() throws java.io.IOException, java.lang.InterruptedException {}

  void c() throws FirstVeryLongExceptionName, SecondVeryLongExceptionName, ThirdVeryLongException {}

  abstract void d() throws FirstVeryLongExceptionName, SecondVeryLongExceptionName, ThirdVeryLon;

  <T extends Exception> void e() throws T {}

  Throws() throws E {}

  void f() throws /* c */ First, Second {}

  void g() throws First /* c */, Second {}

  void h() throws First, Second /* c */ {}
}
