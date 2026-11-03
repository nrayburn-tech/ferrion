/// Before   package
package p;

/// Before   import
import a.B;

/// Class   doc
class A {
  /// Field   doc uses B
  B f;

  /// run   then line comment
  // line
  void a() {}

  /// run   then block comment
  /* block */
  void b() {}

  /// run   then annotation
  @Deprecated
  void c() {}

  @Deprecated
  /// after   annotation
  void d() {}

  /** javadoc */
  /// run   after javadoc
  void f() {}

  /// run   before javadoc
  /** javadoc   two */
  void g() {}

  void h() {
    /// in   body
    int x;
    /// local   class
    class L {}
  }

  enum E {
    /// enum   constant
    X,
    /// second   constant
    Y
  }

  /// before   static init
  static {
  }

  /// field   one
  int a, b;

  /// first   run

  /// second   run
  void i() {}

  ////x first   four
  /// then   three
  void j() {}

  /// then   three
  ////x then   four
  void k() {}

  /// Dangling   end
}
