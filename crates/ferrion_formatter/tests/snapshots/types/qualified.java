// Qualified types, in a bound, where they break as the types rules alone decide: a package and its
// types fill their lines; another chain keeps its head on the first line and puts the rest one
// name per line.
class Fits<T extends Comparable<T> & com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.Some> {}

class Filled<T extends Comparable<T> & com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever.Some.Other> {}

class Types<T extends Comparable<T> & Outer.Middle.Inner.Deepest.EvenDeeper.EvenDeeperStill.YetMoreDepth.AndMoreDepth.AndStillMore.Last.More.EvenMore.AndMore.AndYetMore.AndStill.Moreeeeeeeee.AndTheLastOne> {}

class Arguments<T extends Comparable<T> & com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.Some<A, B>> {}

class ArgumentsBreak<T extends Comparable<T> & com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever.Some<SomeVeryLongTypeArgumentNameNumberOne, B>> {}

class Lowercase<T extends Comparable<T> & abcde.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever.lowercase> {}

class ShortFirstName<T extends Comparable<T> & abc.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever.lowercase> {}

class Mixed<T extends Comparable<T> & com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.Outer.inner.Deeper.Deepest.EvenMore> {}
