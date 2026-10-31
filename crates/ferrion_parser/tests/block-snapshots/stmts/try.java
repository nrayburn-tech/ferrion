{
    try { } catch (A e) { }
    try { } catch (final A | B e) { } finally { }
    try { } finally { }
    try (R r = open(); s) { }
    try (r; this.s;) { } catch (E e) { }
}
