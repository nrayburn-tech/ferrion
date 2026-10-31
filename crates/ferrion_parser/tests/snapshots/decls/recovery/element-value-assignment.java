// An element value is a conditional expression: javac expects `)` after `y`.
@A(x = y = 1)
class B {}

@C(a.b = 1)
class D {}
