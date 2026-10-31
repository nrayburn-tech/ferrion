interface I {}

public interface J<T> extends I, K<T> {
  int CONSTANT = 1;

  void abstractMethod();

  default void defaultMethod() {}

  static void staticMethod() {}

  private void privateMethod() {}
}
