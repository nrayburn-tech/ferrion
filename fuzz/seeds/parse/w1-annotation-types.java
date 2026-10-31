@interface Empty {}

public @interface Info {
  int value() default 1;

  String[] names() default {"a", "b"};

  Class<?> type() default Object.class;

  Empty nested() default @Empty;

  int CONSTANT = 2;

  enum Kind { A, B }
}

// javac rejects these only after parsing.
@interface Generic<T> extends Annotation {}
