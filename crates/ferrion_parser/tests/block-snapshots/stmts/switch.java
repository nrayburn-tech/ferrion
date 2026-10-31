{
    switch (x) {
        case A:
        case B, C:
            f();
            break;
        default:
    }
    switch (x) {
        case A -> f();
        case B, C -> { }
        case D -> throw e;
        case null, default -> { }
    }
    switch (x) {
        case (A) -> { }
        case A: case B -> { }
        default -> { }
    }
    switch (o) { }
}
