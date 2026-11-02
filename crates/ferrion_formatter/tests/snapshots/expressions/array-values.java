// An annotation's array of values that doesn't fit has its values one per line, unless they are
// all narrow enough to fill; they are never all on a line of their own. An element's default
// is laid out as an array initializer.
@SuppressWarnings({"all", "warnings", "unchecked", "unused", "cast", "CheckReturnValue", "NullAway"})
@interface ArrayValues {
  @SuppressWarnings({"all", "warnings", "unchecked", "unused", "cast", "CheckReturnValue", "Null"})
  int a();
  @SuppressWarnings({"all", "warn", "unch", "unu", "cast", "Check", "Null", "aaa", "bbb", "ccc", "ddd", "eee"})
  int b();
  @SuppressWarnings(value = {"all", "warnings", "unchecked", "unused", "cast", "CheckReturnValue", "NullAway"})
  int c();
  @Foo({Aaaaaaaaaaaaaa.class, Bbbbbbbbbbbbbbbbbbbbb.class, Cccccccccccccccccccc.class, Ddddddd.class})
  int d();
  @Foo({@Bar("aaaaaaaaaaaaaaaaaaaaaa"), @Bar("bbbbbbbbbbbbbbbbbbbbbbbbbbb"), @Bar("ccccccccccccccccc")})
  int e();
  @Foo(x = @Bar({"all", "warnings", "unchecked", "unused", "cast", "CheckReturnValue", "NullAwayyyy"}))
  int f();
  @A({
    "a", "b", "c",
    "d", "e", "f"
  })
  int g();
  String[] h() default {"all", "warnings", "unchecked", "unused", "cast", "CheckReturnValue", "NullAway"};
  String[] i() default {"allllllllllll", "warningsssssssssss", "uncheckedddddddd", "unusedddddddddd", "cast", "CheckReturnValue", "NullAway"};
}
