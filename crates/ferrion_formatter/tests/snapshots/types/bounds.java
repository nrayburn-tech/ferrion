// Bounds: broken after `extends`, then filled, breaking before `&`.
class Fits<T extends Comparable<T> & java.io.Serializable & Cloneable & SomeOtherInterface & Abc> {}

class AfterExtends<T extends Comparable<T> & java.io.Serializable & Cloneable & SomeOtherInterface & AAAAAA & BBBBBBB> {}

class Filled<T extends Comparable<T> & java.io.Serializable & Cloneable & SomeOtherInterface & AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA & BBBBBBBBBBBBBBBBBBBBBBBB & CCCCCCCCCCCCCCCCCCCCCCCCCCCCC & D> {}

class BeforeAComma<T extends Comparable<T> & java.io.Serializable & Cloneable & SomeOtherInterface & AAAAAAAAAAAAAAAAAA & BBBBBBBBBBBBBBBBBBB, U> {}

class TheTextAfterCounts<T extends SomeVeryLongBoundTypeNameNumberOneXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX & B> {}

class TheLastFits<T extends SomeVeryLongBoundTypeNameNumberOneXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX & B & C> {}

class Two<T extends Comparable<T> & java.io.Serializable, U extends Comparable<U> & java.io.Serializable> {}
