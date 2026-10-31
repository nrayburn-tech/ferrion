{
    switch (o) {
        case String s -> { }
        case final Integer i when i > 0 -> { }
        case Point(var x, _) -> { }
        case Box(Point(int x, int y)) when x == y -> { }
        case Empty() -> { }
        case int i -> { }
        case FOO -> { }
        case foo(1) -> { }
        case _ -> { }
    }
}
