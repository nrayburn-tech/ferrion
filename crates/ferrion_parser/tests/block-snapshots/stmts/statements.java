{
    ;
    { }
    final int x = 1, y[] = {2};
    f();
    if (a) f(); else if (b) g(); else h();
    if (a) { } else { }
    while (a) ;
    do ; while (a);
    for (;;) ;
    for (int i = 0, j = 0; i < j; i++, j--) ;
    for (final int i : xs) ;
    outer: inner: break outer;
    continue;
    continue outer;
    return;
    return x;
    throw e;
    assert a;
    assert a : b;
    synchronized (a) { }
    final class C { }
    abstract class D { }
    interface I { }
    enum E { }
    @A int z;
}
