use std::marker::PhantomData;
#[derive(Debug, Clone, Copy)]
struct OrderId(u32);

#[derive(Debug, Clone)]
struct CustomerName(String);

#[derive(Debug, Clone, Copy)]
struct Price(f64);
#[derive(Debug)]
struct New;

#[derive(Debug)]
struct Paid;

#[derive(Debug)]
struct Shipped;
#[derive(Debug)]
struct Order<State> {
    id: OrderId,
    customer: CustomerName,
    price: Price,
    state: PhantomData<State>,
}
impl Order<New> {
    fn new(id: OrderId, customer: CustomerName, price: Price) -> Self {
        Order {
            id,
            customer,
            price,
            state: PhantomData,
        }
    }
    fn pay(self) -> Order<Paid> {
        println!("Замовлення №{} оплачено.", self.id.0);

        Order {
            id: self.id,
            customer: self.customer,
            price: self.price,
            state: PhantomData,
        }
    }
}
impl Order<Paid> {
    fn ship(self) -> Order<Shipped> {
        println!("Замовлення №{} відправлено.", self.id.0);

        Order {
            id: self.id,
            customer: self.customer,
            price: self.price,
            state: PhantomData,
        }
    }
}
impl Order<Shipped> {
    fn complete(&self) {
        println!("Замовлення №{} доставлено клієнту.", self.id.0);
    }
}
fn main() {
    let order = Order::<New>::new(
        OrderId(1),
        CustomerName("Іван Петренко".to_string()),
        Price(1200.0),
    );

    println!("Створено нове замовлення.");

    let paid_order = order.pay();

    let shipped_order = paid_order.ship();

    shipped_order.complete();
}
