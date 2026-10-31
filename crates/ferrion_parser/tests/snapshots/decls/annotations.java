@Marker
@Single(1)
@Pairs(a = 1, b = "two")
@Arrays({1, 2, 3})
@EmptyArray({})
@TrailingComma({1, 2,})
@OnlyComma({,})
@Nested(@Marker)
@NestedInArrays({@A, @B(x = {@C})})
@a.b.Qualified
@Empty()
// javac rejects a mix of values and pairs only after parsing.
@Mixed(1, x = 2)
class A {}
