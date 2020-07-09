### `actix_send` is an actor pattern loosely based on the design of [actix](https://crates.io/crates/actix).

### Difference from actix:
- Can run on any async runtime.
- Message is typed into actor and use static dispatch while actix using dynamic dispatch for message using trait object.(Dynamic dispatch is also provided by passing boxed future directly to actor an no message type boilerplate is required.)
- Rely heavily on proc macro to achieve static dispatch mentioned above for multiple messages on one actor and it also brings some boilerplate actix doesn't have.


```javascript

const array = [1, 2, 3 , 4 ,5];

let prev;
todos.filter(todo => todo.name == "Jim")
     .map(todo => todo.complete)
     .map(complete => complete ? 1:0)
     .reduce((total, current) => total + current, 0)   
}



function test(a, b) {
    return a + b;
};


class A {

    test(b) {
        return this.a + b;
    }
}


```